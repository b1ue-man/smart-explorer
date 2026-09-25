package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.BuildConfig
import app.smartexplorer.android.api.Roots
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.core.VolumeInfo
import java.io.File
import kotlinx.serialization.builtins.ListSerializer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** G4: `sys.*`, `loc.*`, `task.*` and the Google Drive configuration through the real bridge. */
@RunWith(AndroidJUnit4::class)
class SystemTaskTest {
    @Test
    fun systemInfoVolumesHostStateAndErrorLog() = coreTest {
        val info = Api.obj("sys.info")
        assertEquals(BuildConfig.VERSION_NAME, info.text("coreVersion"))
        assertEquals(File(appContext.filesDir, "smart_explorer").absolutePath, info.text("dataDir"))
        assertTrue(info.text("cacheDir").startsWith(appContext.cacheDir.absolutePath))

        val volumes = Core.json.encodeToJsonElement(ListSerializer(VolumeInfo.serializer()), Volumes.all())
        Api.call("sys.volumes", args("volumes" to volumes))
        Api.call(
            "sys.hostState",
            args("powerSave" to false, "metered" to false, "wifi" to true, "charging" to true, "foreground" to false),
        )

        Api.call("sys.clearErrors")
        // A task that fails lands in the error log (api.md §4.1).
        val missing = File(Fixture.dir(Volumes.primary(), "system"), "fehlt").absolutePath
        val failed = Api.await(Api.start("fs.properties", args("locations" to listOf(missing))))
        assertEquals("failed", failed.state)
        val errors = Api.objects("sys.errors")
        assertTrue("Fehlerprotokoll leer nach fehlgeschlagenem Task", errors.isNotEmpty())
        errors.forEach { entry -> assertTrue(entry.long("timeMs") > 0 && entry.text("message").isNotEmpty()) }
        Api.call("sys.clearErrors")
        assertTrue(Api.objects("sys.errors").isEmpty())
        val crash = Api.obj("sys.crashLog")
        TaskReport.note("sys.crashLog", "${crash.text("text").length} Zeichen")
    }

    @Test
    fun rootsAndFavorites() = coreTest {
        val roots: Roots = Api.get("loc.roots")
        val primary = Volumes.primary()
        assertTrue("Primäres Volume fehlt in loc.roots.storage", roots.storage.any { it.location == primary.path && it.kind == "storage" })
        val sd = Volumes.sdCard()
        assertTrue("SD-Karte fehlt in loc.roots.storage", roots.storage.any { it.location == sd.path && it.removable })
        val trash = assertNotNullReturn(roots.trash, "loc.roots.trash")
        assertEquals("trash", trash.kind)

        val folder = Fixture.dir(primary, "favorites", "Lieblingsordner").absolutePath
        assertTrue(Api.obj("loc.toggleFavorite", args("location" to folder)).bool("favorite"))
        assertTrue(Api.obj("loc.isFavorite", args("location" to folder)).bool("favorite"))
        assertTrue(Api.get<Roots>("loc.roots").favorites.any { it.location == folder })
        assertFalse(Api.obj("loc.toggleFavorite", args("location" to folder)).bool("favorite"))
        assertFalse(Api.obj("loc.isFavorite", args("location" to folder)).bool("favorite"))
        // App-internal places never become desktop favorites.
        Api.failure("loc.toggleFavorite", args("location" to trash.location), "invalid")
        Api.failure("loc.toggleFavorite", args("location" to "zip://$folder/a.zip!/"), "invalid")
        // Listing a folder records it in "Zuletzt".
        Api.listing(folder)
        assertTrue(Api.get<Roots>("loc.roots").recent.any { it.location == folder })
    }

    @Test
    fun taskListGetCancelAndClear() = coreTest {
        val root = Fixture.dir(Volumes.primary(), "tasks")
        repeat(400) { index -> File(root, "d$index/sub").mkdirs() }
        val scan = Api.start("scan.start", args("location" to root.absolutePath, "filter" to null, "showHidden" to true))
        Api.call("task.cancel", args("id" to scan))
        val canceled = Api.await(scan)
        assertTrue("Scan-Abbruch: ${canceled.state}", canceled.state == "canceled" || canceled.state == "done")
        val second = Api.start("scan.start", args("location" to root.absolutePath, "filter" to null, "showHidden" to true))
        Api.call("task.cancelAll", args("kind" to "scan"))
        assertFalse(Api.await(second).isActive)
        val listed = Core.json.decodeFromJsonElement(ListSerializer(TaskInfo.serializer()), Api.call("task.list"))
        assertTrue(listed.any { it.id == scan } && listed.any { it.id == second })
        // task.clear drops finished tasks once their final snapshot was delivered.
        val deadline = System.currentTimeMillis() + 15_000
        var remaining = listed
        while (System.currentTimeMillis() < deadline) {
            Api.call("task.clear")
            remaining = Core.json.decodeFromJsonElement(ListSerializer(TaskInfo.serializer()), Api.call("task.list"))
            if (remaining.none { it.id == scan || it.id == second }) break
            kotlinx.coroutines.delay(500)
        }
        assertTrue("task.clear ließ fertige Tasks stehen: $remaining", remaining.none { it.id == scan || it.id == second })
        Api.failure("task.get", args("id" to scan), "not_found")
    }

    @Test
    fun googleDriveConfiguration() = coreTest {
        Api.obj("gdrive.status")
        Api.call("gdrive.configure", args("clientId" to "task-suite.apps.googleusercontent.com", "clientSecret" to null))
        val configured = Api.obj("gdrive.status")
        assertTrue(configured.bool("clientConfigured"))
        assertEquals("task-suite.apps.googleusercontent.com", configured.text("clientId"))
        Api.call("gdrive.signOut")
        assertFalse(Api.obj("gdrive.status").bool("signedIn"))
    }

    private fun <T : Any> assertNotNullReturn(value: T?, what: String): T {
        assertNotNull("$what fehlt", value)
        return value!!
    }
}
