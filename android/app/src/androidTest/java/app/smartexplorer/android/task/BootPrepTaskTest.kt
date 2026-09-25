package app.smartexplorer.android.task

import android.content.Context
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.service.BackgroundController
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G4 boot preparation (own `am instrument` run): switch to the persistent background mode and
 * store it synchronously, because the process ends with the instrumentation. The host script then
 * reboots the device and checks that only the specialUse background service started.
 */
@RunWith(AndroidJUnit4::class)
class BootPrepTaskTest {
    @Test
    fun persistentModeIsStoredForTheBootBroadcast() {
        AppPrefs.setOnboardingDone(true)
        AppPrefs.setBgMode(BackgroundController.MODE_PERSISTENT)
        // AppPrefs writes with apply(); commit the same keys so they survive the process end.
        val stored = appContext.getSharedPreferences("app_prefs", Context.MODE_PRIVATE)
            .edit()
            .putString("bg_mode", BackgroundController.MODE_PERSISTENT)
            .putBoolean("onboarding_done", true)
            .commit()
        assertTrue("Einstellungen nicht gespeichert", stored)
        assertEquals(BackgroundController.MODE_PERSISTENT, AppPrefs.bgMode.value)
    }
}
