package app.smartexplorer.android.task

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import androidx.core.content.FileProvider
import androidx.test.core.app.ActivityScenario
import androidx.test.espresso.intent.Intents
import androidx.test.espresso.intent.matcher.IntentMatchers
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.MainActivity
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.system.LocalFileProvider
import app.smartexplorer.android.system.Opener
import app.smartexplorer.android.system.ShareIntentHandler
import java.io.File
import org.hamcrest.CoreMatchers.allOf
import org.hamcrest.CoreMatchers.equalTo
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G4 hand-over to other apps (Espresso-Intents, the target activities are stubbed) and receiving a
 * SEND share: `fs.open` + LocalFileProvider (read, and "w" truncates the original),
 * `fs.materialize` + share sheet, SEND with a content URI → descriptor → `fs.import`.
 */
@RunWith(AndroidJUnit4::class)
class IntentsTaskTest {
    private fun stubExternalActivities() {
        Intents.intending(IntentMatchers.anyIntent()).respondWith(Instrumentation.ActivityResult(Activity.RESULT_OK, null))
    }

    @Test
    fun openLocalFileThroughTheLocalFileProvider() = coreTest {
        AppPrefs.setOnboardingDone(true)
        val dir = Fixture.dir(Volumes.primary(), "intents", "open")
        val file = Fixture.write(File(dir, "lesen.txt"), "Ursprünglicher, längerer Inhalt")
        val target = Api.obj("fs.open", args("location" to file.absolutePath))
        assertEquals(file.absolutePath, target.text("localPath"))
        Api.failure("fs.open", args("location" to dir.absolutePath), "invalid")
        val uri = LocalFileProvider.uriFor(appContext, file.absolutePath)
            ?: throw AssertionError("Keine Content-URI für ${file.path}")

        val scenario = ActivityScenario.launch(MainActivity::class.java)
        Intents.init()
        try {
            stubExternalActivities()
            var outcome: Opener.Outcome? = null
            scenario.onActivity { activity -> outcome = Opener.open(activity, target.text("localPath"), target.textOrNull("mime"), false) }
            assertEquals(Opener.Outcome.Started, outcome)
            Intents.intended(
                allOf(
                    IntentMatchers.hasAction(Intent.ACTION_VIEW),
                    IntentMatchers.hasData(uri),
                    IntentMatchers.hasFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION, Intent.FLAG_GRANT_WRITE_URI_PERMISSION),
                ),
            )
        } finally {
            Intents.release()
            scenario.close()
        }
        val resolver = appContext.contentResolver
        val read = resolver.openInputStream(uri)?.use { String(it.readBytes()) }
        assertEquals("Ursprünglicher, längerer Inhalt", read)
        // Writes of the other app land in the original; "w" truncates (no stale tail).
        resolver.openOutputStream(uri, "w")?.use { it.write("kurz".toByteArray()) } ?: throw AssertionError("Provider nicht beschreibbar")
        assertEquals("kurz", file.readText())
    }

    @Test
    fun shareMaterializedFilesThroughTheShareSheet() = coreTest {
        AppPrefs.setOnboardingDone(true)
        val dir = Fixture.dir(Volumes.primary(), "intents", "share")
        val files = listOf(Fixture.write(File(dir, "eins.txt"), "1"), Fixture.write(File(dir, "zwei.txt"), "2")).map { it.absolutePath }
        val paths = Api.runTask("fs.materialize", args("locations" to files)).resultObj().texts("paths")
        assertEquals("Lokale Orte bleiben unverändert", files.toSet(), paths.toSet())

        val scenario = ActivityScenario.launch(MainActivity::class.java)
        Intents.init()
        try {
            stubExternalActivities()
            var outcome: Opener.Outcome? = null
            scenario.onActivity { activity -> outcome = Opener.share(activity, paths) }
            assertEquals(Opener.Outcome.Started, outcome)
            Intents.intended(
                allOf(
                    IntentMatchers.hasAction(Intent.ACTION_CHOOSER),
                    IntentMatchers.hasExtra(equalTo(Intent.EXTRA_INTENT), IntentMatchers.hasAction(Intent.ACTION_SEND_MULTIPLE)),
                ),
            )
        } finally {
            Intents.release()
            scenario.close()
        }
    }

    @Test
    fun receiveASharedFileIntoAFolderOnTheSdCard() = coreTest {
        AppPrefs.setOnboardingDone(true)
        val source = Fixture.write(File(appContext.cacheDir, "share/empfang-quelle.txt"), "geteilt von einer anderen App")
        val uri = FileProvider.getUriForFile(appContext, appContext.packageName + ".files", source)
        val intent = Intent(appContext, MainActivity::class.java).apply {
            action = Intent.ACTION_SEND
            type = "text/plain"
            putExtra(Intent.EXTRA_STREAM, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        val target = Fixture.dir(Volumes.sdCard(), "intents", "empfang")
        ShareIntentHandler.discard()
        val scenario = ActivityScenario.launch<MainActivity>(intent)
        try {
            waitFor("geteilte Datei geöffnet", 20_000) { ShareIntentHandler.pending.value is ShareIntentHandler.Incoming.Ready }
            val taskId = ShareIntentHandler.importInto(target.absolutePath)
            TaskReport.called("fs.import", "ok")
            val done = Api.await(taskId)
            assertEquals("fs.import: ${done.message} ${done.errors}", "done", done.state)
            assertEquals("geteilt von einer anderen App", File(target, "empfang-quelle.txt").readText())
        } finally {
            scenario.close()
            source.delete()
        }
    }
}
