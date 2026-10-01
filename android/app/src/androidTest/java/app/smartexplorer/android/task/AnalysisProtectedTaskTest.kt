package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.system.StorageStatsAccess
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Storage analysis on the device (docs/superpowers/plans/2026-10-01-android-hintergrund-analyse,
 * B1, B2, B5, M13): a whole-volume analysis treats other apps' `Android/data|obb` as protected
 * (no read issues, complete result), a protected root ends complete instead of failed, and the
 * duplicate search finds duplicates beyond the former 200-candidate cap.
 */
@RunWith(AndroidJUnit4::class)
class AnalysisProtectedTaskTest {
    @Test
    fun volumeAnalysisTreatsOtherAppsFoldersAsProtected() = coreTest(timeoutMs = 25 * 60_000L) {
        val volume = Volumes.primary()
        val figures = StorageStatsAccess.figuresFor(appContext, volume.path)
        val platform = buildMap<String, Any> {
            figures?.volumeUsedBytes?.let { put("volumeUsedBytes", it) }
            figures?.otherAppsBytes?.let { put("otherAppsBytes", it) }
        }
        val analysis = Api.runTask(
            "analyze.start",
            args("location" to volume.path, "platform" to platform),
            timeoutMs = 20 * 60_000L,
        )
        assertEquals("Analyse des Volumes nicht fertig: ${analysis.message}", "done", analysis.state)
        val issues = Api.obj("analyze.issues", args("taskId" to analysis.id))
        val text = issues.text("text")
        assertFalse("Geschützte App-Ordner als Lesefehler gemeldet: $text", text.contains("/Android/data") || text.contains("/Android/obb"))
        issues.long("protectedCount")
        val root = Api.obj("analyze.node", args("taskId" to analysis.id, "path" to emptyList<String>()))
        val kinds = root.objects("children").associate { it.text("name") to it.text("kind") }
        assertEquals("Android ist kein Ordner: $kinds", "dir", kinds["Android"])
        if (issues.int("count") == 0 && figures?.volumeUsedBytes != null) {
            assertTrue("Restzeile „≈ Nicht einzeln erfasst“ fehlt: $kinds", kinds.values.contains("rest"))
            assertTrue("Wurzelgröße kleiner als gemessen", root.long("size") >= root.long("measured"))
        }
        val android = Api.obj("analyze.node", args("taskId" to analysis.id, "path" to listOf("Android")))
        TaskReport.note(
            "analyze.protected",
            "issues=$issues platform=$platform android=${android.objects("children").map { it.text("name") + ":" + it.text("kind") }}",
        )

        // A protected root (other apps' OBB folder) ends complete instead of failed.
        val obb = File(volume.path, "Android/obb").absolutePath
        val protectedRoot = Api.runTask("analyze.start", args("location" to obb))
        assertEquals("Geschützte Wurzel endete nicht vollständig: ${protectedRoot.message}", "done", protectedRoot.state)
    }

    @Test
    fun duplicateSearchFindsDuplicatesBeyondTheFormerCandidateCap() = coreTest(timeoutMs = 15 * 60_000L) {
        val root = Fixture.dir(Volumes.primary(), "duplikate-${System.currentTimeMillis()}")
        // 260 pairs of equal-sized small files plus one large pair: the former search kept only the
        // 200 largest candidates, so most small pairs were never compared.
        val pairs = 260
        for (index in 0 until pairs) {
            Fixture.bytes(File(root, "a/klein-$index.bin"), 4096, index)
            Fixture.bytes(File(root, "b/klein-$index.bin"), 4096, index)
        }
        Fixture.bytes(File(root, "a/gross.bin"), 2 * 1024 * 1024, 7_777)
        Fixture.bytes(File(root, "b/gross.bin"), 2 * 1024 * 1024, 7_777)
        Fixture.bytes(File(root, "a/einzeln.bin"), 4096, 99_999)

        val search = Api.runTask("reclaim.start", args("location" to root.absolutePath, "minSize" to 1024))
        assertEquals("done", search.state)
        val groups: List<DuplicateGroup> = Api.get("reclaim.groups", args("taskId" to search.id))
        assertEquals("Duplikatgruppen", pairs + 1, groups.size)
        assertTrue("Gruppe mit mehr als zwei Kopien: $groups", groups.all { it.items.size == 2 })
        val summary = Api.obj("reclaim.summary", args("taskId" to search.id))
        assertEquals("Kandidaten", (2 * pairs + 3).toLong(), summary.long("candidates"))
        assertEquals("Gruppen in der Zusammenfassung", (pairs + 1).toLong(), summary.long("groups"))
        assertEquals("Fehler: ${summary.textOrNull("errorText")}", 0L, summary.long("errorCount"))
        assertTrue("Grenze erreicht: ${summary.textOrNull("limit")}", summary.textOrNull("limit").isNullOrEmpty())
        root.deleteRecursively()
    }
}
