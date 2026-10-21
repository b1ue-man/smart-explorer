package app.smartexplorer.android.launcher

import android.accessibilityservice.AccessibilityService
import app.smartexplorer.android.launcher.bridge.BridgeProtocol
import app.smartexplorer.android.launcher.gestures.GestureAvailability
import app.smartexplorer.android.launcher.gestures.SeGestures
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The two gestures on the JVM without Android (U40-18): when the accessibility service counts as switched on, the texts and the action numbers. */
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
    }

    @Test
    fun another_service_or_an_empty_setting_leaves_the_gestures_off() {
        assertFalse(GestureAvailability.listed(null, pkg, service))
        assertFalse(GestureAvailability.listed("", pkg, service))
        assertFalse(GestureAvailability.listed("com.other/com.other.Service", pkg, service))
        // Same class name in another package is another service.
        assertFalse(GestureAvailability.listed("com.other/$service", pkg, service))
        assertFalse(GestureAvailability.listed("$pkg/app.smartexplorer.android.screen.ScreenHostService", pkg, service))
        // A prefix of the entry is no match.
        assertFalse(GestureAvailability.listed("$pkg/${service}X", pkg, service))
    }

    @Test
    fun the_messages_and_the_global_actions_are_as_specified() {
        assertEquals("Gesten sind aus. Schalte die Bedienungshilfe in den Einstellungen ein.", SeGestures.OFF_TEXT)
        assertEquals("Die Geste ist gerade nicht verfügbar.", SeGestures.UNAVAILABLE_TEXT)
        // GLOBAL_ACTION_NOTIFICATIONS is 4 and GLOBAL_ACTION_LOCK_SCREEN is 8 (API 28, A38); the bridge names are the wire form.
        assertEquals(4, AccessibilityService.GLOBAL_ACTION_NOTIFICATIONS)
        assertEquals(8, AccessibilityService.GLOBAL_ACTION_LOCK_SCREEN)
        assertEquals("notifications", BridgeProtocol.ACTION_NOTIFICATIONS)
        assertEquals("lockScreen", BridgeProtocol.ACTION_LOCK_SCREEN)
        assertEquals("accessibility_off", BridgeProtocol.REASON_ACCESSIBILITY_OFF)
        assertEquals("unsupported", BridgeProtocol.REASON_UNSUPPORTED)
    }
}
