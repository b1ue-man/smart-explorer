package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.service.BackgroundController
import app.smartexplorer.android.service.BackgroundService
import app.smartexplorer.android.system.Notifications
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Background reachability (docs/superpowers/plans/2026-10-01-android-hintergrund-analyse, A1–A3,
 * M10/M11): runs after the phone joined the desktop's Room (G5), so Share is set up and the Share
 * server of the suite (`SE_SHARE_IDLE_KEEPALIVE_SECS=30`) negotiates the idle keepalive.
 *
 * The instrumentation has no visible activity, so the app is "in the background" for the core
 * (`sys.hostState.foreground = false`): the Share worker must be in the server-driven idle mode.
 * The host script then forces Doze and lets the desktop CLI open a session to the phone.
 */
@RunWith(AndroidJUnit4::class)
class BackgroundReachTaskTest {
    @Test
    fun reachableServiceRunsInPeriodicModeOnceShareIsSetUp() = coreTest {
        AppPrefs.setOnboardingDone(true)
        AppPrefs.setBgMode(BackgroundController.MODE_PERIODIC)
        AppPrefs.setShareReachable(true)
        BackgroundController.apply(appContext)
        waitFor("Share läuft (Room aus G5)", 120_000) { Api.obj("share.status").bool("running") }
        waitFor("„Share erreichbar“-Dienst im Modus Periodisch", 60_000) {
            BackgroundService.isRunning && activeNotification(Notifications.ID_BACKGROUND) != null
        }
        val notification = activeNotification(Notifications.ID_BACKGROUND)!!.notification
        val title = notification.extras.getCharSequence(android.app.Notification.EXTRA_TITLE)?.toString()
        assertEquals("Share erreichbar", title)
        assertTrue("Erreichbarkeit ist nicht als gespeichert markiert", AppPrefs.shareRunning.value)
        TaskReport.note("reach.service", "title=$title")
    }

    @Test
    fun idleKeepaliveIsNegotiatedAndTheProbeAnswers() = coreTest {
        waitFor("Share-Server verbunden", 120_000) { Api.obj("share.status").bool("connected") }
        var power = Api.obj("share.status").field("power").obj()
        waitFor("Ruhemodus vom Server bestätigt (App im Hintergrund)", 120_000) {
            power = Api.obj("share.status").field("power").obj()
            power.textOrNull("idleSupported") == "true" && power.bool("idleActive")
        }
        assertEquals("Keepalive-Abstand des Test-Servers", 30, power.int("keepaliveSecs"))
        val probe = Api.obj("share.wake", args("networkChanged" to false))
        assertTrue("Probe ohne Erfolg: $probe", probe.bool("ok"))
        val changed = Api.obj("share.wake", args("networkChanged" to true))
        assertTrue("Probe nach Netzwechsel ohne Erfolg: $changed", changed.bool("ok"))
        // At least one server keepalive arrives within two intervals and is answered.
        fun contact(power: kotlinx.serialization.json.JsonObject): Long =
            power.textOrNull("lastServerContactMs")?.toLongOrNull() ?: 0L
        val before = contact(power)
        waitFor("Server-Keepalive beantwortet", 90_000) {
            contact(Api.obj("share.status").field("power").obj()) > before
        }
        val phone = Api.obj("share.status").field("identity").obj().text("deviceId")
        // The host script reads the device id for the desktop's `reach` check.
        val marker = File("/sdcard/SmartExplorerTask/reach/phone")
        marker.parentFile?.mkdirs()
        marker.writeText(phone)
        TaskReport.note("reach.power", "power=$power probe=$probe changed=$changed")
    }
}
