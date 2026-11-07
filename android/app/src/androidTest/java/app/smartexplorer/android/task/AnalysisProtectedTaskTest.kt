package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.api.AnalyzeApi
import app.smartexplorer.android.system.StorageStatsAccess
import java.io.File
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Storage analysis on the device (docs/superpowers/plans/2026-10-01-android-hintergrund-analyse,
 * B1, B2, B5, B6, M13, M14): a whole-volume analysis treats other apps' `Android/data|obb` as
 * protected (no read issues, complete result) and, with usage access (the suite grants
 * `GET_USAGE_STATS`), lists the installed apps with the own app among them; a protected root ends
 * complete instead of failed.
 */
@RunWith(AndroidJUnit4::class)
class AnalysisProtectedTaskTest {
    @Test
    fun volumeAnalysisTreatsOtherAppsFoldersAsProtected() = coreTest(timeoutMs = 25 * 60_000L) {
        val volume = Volumes.primary()
        val figures = StorageStatsAccess.figuresFor(appContext, volume.path)
        val platform = figures?.platform?.let { AnalyzeApi.platformJson(it) }
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
            assertTrue("Restzeile („≈ Nicht einzeln erfasst“ bzw. „≈ System und Sonstiges“) fehlt: $kinds", kinds.values.contains("rest"))
            assertTrue("Wurzelgröße kleiner als gemessen", root.long("size") >= root.long("measured"))
        }
        val android = Api.obj("analyze.node", args("taskId" to analysis.id, "path" to listOf("Android")))
        TaskReport.note(
            "analyze.protected",
            "issues=$issues volumeUsed=${figures?.volumeUsedBytes} otherApps=${figures?.otherAppsBytes} " +
                "apps=${figures?.apps?.size} android=${android.objects("children").map { it.text("name") + ":" + it.text("kind") }}",
        )

        // A protected root (other apps' OBB folder) ends complete instead of failed.
        val obb = File(volume.path, "Android/obb").absolutePath
        val protectedRoot = Api.runTask("analyze.start", args("location" to obb))
        assertEquals("Geschützte Wurzel endete nicht vollständig: ${protectedRoot.message}", "done", protectedRoot.state)

        checkAppList(analysis.id, root, figures)
    }

    /**
     * B6/M14: with usage access the root of the internal storage has "≈ Apps (laut Android)"; it
     * opens the app list (no place in "Dateien"), the own app is listed with size = app + data, and
     * other apps' data no longer appears as an extra row under `Android/data`.
     */
    private suspend fun checkAppList(taskId: String, root: JsonObject, figures: StorageStatsAccess.Figures?) {
        assertTrue(
            "Zugriff auf Nutzungsdaten fehlt (appops set <Paket> GET_USAGE_STATS allow)",
            StorageStatsAccess.hasUsageAccess(appContext),
        )
        assertTrue("Keine App-Liste von Android: ${figures?.apps}", !figures?.apps.isNullOrEmpty())
        val children = root.objects("children")
        val appsRow = children.firstOrNull { it.text("kind") == "apps" }
            ?: throw AssertionError("Zeile „≈ Apps (laut Android)“ fehlt: ${children.map { it.text("name") + ":" + it.text("kind") }}")
        assertTrue("Apps-Zeile ist nicht zu öffnen: $appsRow", appsRow.bool("isDir"))
        assertTrue("Apps-Zeile ohne Größe: $appsRow", appsRow.long("size") > 0)
        val list = Api.obj("analyze.node", args("taskId" to taskId, "path" to listOf(appsRow.text("name"))))
        assertEquals("apps", list.text("kind"))
        assertNull("App-Liste hat einen Ort in „Dateien“: ${list["location"]}", list.textOrNull("location"))
        val apps = list.objects("children")
        val own = apps.firstOrNull { it.textOrNull("package") == appContext.packageName }
            ?: throw AssertionError("Eigene App fehlt in der App-Liste: ${apps.take(20)}")
        assertEquals("app", own.text("kind"))
        assertTrue("Eigene App ohne Größe: $own", own.long("size") > 0)
        assertEquals("Größe ≠ App + Daten: $own", own.long("appBytes") + own.long("dataBytes"), own.long("size"))
        assertTrue("Cache größer als Daten: $own", own.long("cacheBytes") <= own.long("dataBytes"))
        // Other apps' data is inside the apps now: no extra "laut Android" row under Android/data.
        Api.attempt("analyze.node", args("taskId" to taskId, "path" to listOf("Android", "data"))).getOrNull()?.let { data ->
            val rows = data.obj().objects("children")
            assertFalse("„Weitere App-Daten“ trotz App-Liste: $rows", rows.any { it.text("kind") == "protected" && !it.bool("isDir") })
        }
        TaskReport.note("analyze.apps", "apps=${apps.size} row=${appsRow.long("size")} own=$own")
    }
}
