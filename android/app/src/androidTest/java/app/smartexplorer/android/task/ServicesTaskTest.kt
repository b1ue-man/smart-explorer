package app.smartexplorer.android.task

import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import app.smartexplorer.android.MainActivity
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.service.BackgroundController
import app.smartexplorer.android.service.BackgroundService
import app.smartexplorer.android.system.Notifications
import java.io.File
import kotlinx.coroutines.delay
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G4 services: the persistent background service with its notification and [Pausieren] action,
 * surviving the Home key, and a transfer that keeps running (task service + notification) after
 * the UI was left with the Home key (UiAutomator). `onTimeout` of the dataSync service cannot be
 * provoked on an emulator (explicit exception of the suite).
 */
@RunWith(AndroidJUnit4::class)
class ServicesTaskTest {
    private val device: UiDevice
        get() = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

    @Test
    fun persistentServiceNotificationPauseActionAndHomeKey() = coreTest {
        AppPrefs.setOnboardingDone(true)
        AppPrefs.setBgMode(BackgroundController.MODE_PERSISTENT)
        val scenario = ActivityScenario.launch(MainActivity::class.java)
        try {
            waitFor("Dauerbetrieb-Dienst mit Benachrichtigung", 30_000) {
                BackgroundService.isRunning && activeNotification(Notifications.ID_BACKGROUND) != null
            }
            val notification = activeNotification(Notifications.ID_BACKGROUND)!!.notification
            val pause = notification.actions?.firstOrNull { it.title?.toString() == "Pausieren" }
                ?: throw AssertionError("Aktion [Pausieren] fehlt: ${notification.actions?.map { it.title }}")
            pause.actionIntent.send()
            waitFor("Pause über die Benachrichtigung", 30_000) { Api.obj("bg.status").bool("paused") }
            Api.call("bg.resume")

            device.pressHome()
            delay(4_000)
            assertTrue("Dienst nach der Home-Taste beendet", BackgroundService.isRunning)
            assertNotNull("Benachrichtigung nach der Home-Taste weg", activeNotification(Notifications.ID_BACKGROUND))
        } finally {
            AppPrefs.setBgMode(BackgroundController.MODE_PERIODIC)
            BackgroundController.apply(appContext)
            scenario.close()
        }
        waitFor("Dauerbetrieb-Dienst beendet nach Wechsel auf Periodisch", 30_000) { !BackgroundService.isRunning }
    }

    @Test
    fun transferKeepsRunningAfterTheHomeKey() = coreTest(timeoutMs = 20 * 60_000L) {
        AppPrefs.setOnboardingDone(true)
        val remote = Servers.freshDir(Servers.sftp(), "home-taste")
        val local = Fixture.dir(Volumes.primary(), "services")
        val big = Fixture.big(File(local, "gross.bin"), 256, 42)
        val scenario = ActivityScenario.launch(MainActivity::class.java)
        try {
            val id = Api.start(
                "fs.transfer",
                args("sources" to listOf(big.absolutePath), "targetDir" to remote, "mode" to "copy", "conflict" to "keepBoth"),
            )
            var seenWhileRunning = false
            waitFor("Übertragungs-Benachrichtigung während des Laufs", 60_000) {
                if (activeNotification(Notifications.ID_TRANSFERS) != null) seenWhileRunning = Api.task(id).isActive
                seenWhileRunning || !Api.task(id).isActive
            }
            assertTrue("Übertragung war fertig, bevor der Dienst startete (Datei zu klein?)", seenWhileRunning)
            device.pressHome()
            delay(2_000)
            if (Api.task(id).isActive) {
                assertNotNull("Übertragungsdienst nach der Home-Taste weg", activeNotification(Notifications.ID_TRANSFERS))
            }
            val done = Api.await(id, timeoutMs = 15 * 60_000L)
            assertEquals("Übertragung nach der Home-Taste: ${done.message}", "done", done.state)
            assertEquals(big.length(), Api.child(remote, "gross.bin").size)
            waitFor("Übertragungsdienst beendet sich nach dem Ende", 60_000) { activeNotification(Notifications.ID_TRANSFERS) == null }
        } finally {
            scenario.close()
            big.delete()
        }
    }
}
