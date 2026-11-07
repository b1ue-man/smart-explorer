package app.smartexplorer.android.launcher

import app.smartexplorer.android.launcher.gestures.GestureAvailability
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The gesture switch on the JVM without Android (U40-18): when the accessibility service counts as switched on. */
class SeGesturesTest {
    private val pkg = "app.smartexplorer.android"
    private val service = GestureAvailability.SERVICE_CLASS

    @Test
    fun the_service_is_listed_in_the_full_and_in_the_short_form_without_regard_to_case() {
        assertTrue(GestureAvailability.listed("$pkg/$service", pkg, service))
        assertTrue(GestureAvailability.listed("$pkg/.screen.ScreenControlService", pkg, service))
        assertTrue(GestureAvailability.listed("APP.SMARTEXPLORER.ANDROID/APP.SMARTEXPLORER.ANDROID.SCREEN.SCREENCONTROLSERVICE", pkg, service))
        assertTrue(GestureAvailability.listed("com.other/com.other.Service:$pkg/$service", pkg, service))
        assertTrue(GestureAvailability.listed("$pkg/$service:com.other/com.other.Service", pkg, service))
        // Another service or an empty setting leaves the gestures off (the former another_service_or_an_empty_setting_leaves_the_gestures_off).
        assertFalse(GestureAvailability.listed(null, pkg, service))
        assertFalse(GestureAvailability.listed("", pkg, service))
        assertFalse(GestureAvailability.listed("com.other/com.other.Service", pkg, service))
        // Same class name in another package is another service.
        assertFalse(GestureAvailability.listed("com.other/$service", pkg, service))
        assertFalse(GestureAvailability.listed("$pkg/app.smartexplorer.android.screen.ScreenHostService", pkg, service))
        // A prefix of the entry is no match.
        assertFalse(GestureAvailability.listed("$pkg/${service}X", pkg, service))
    }
}
