package app.smartexplorer.android.launcher.gestures

import android.content.Context
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.geometry.Offset
import app.smartexplorer.android.launcher.bridge.BridgeProtocol
import app.smartexplorer.android.launcher.se.SeController
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * The two gestures that reach into the system (DSK-M13, U40-18): „Wischen nach unten“ opens the notification shade,
 * „Doppeltippen“ on the empty background locks the screen. Both run through the app's accessibility service in the main
 * process; the launcher sends `GLOBAL_ACTION` over the bridge. Without the service they do nothing but say so, once per run.
 */
class SeGestures(private val se: SeController, private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)) {
    /** The handler of „Doppeltippen“: it counts only where [emptyAt] says the background is bare (no item, no dock, no widget). */
    fun doubleTap(context: Context, emptyAt: (Offset) -> Boolean): GestureHandler = GestureHandler { modifier ->
        modifier.pointerInput(Unit) {
            detectTapGestures(onDoubleTap = { at -> if (emptyAt(at)) fire(context, BridgeProtocol.ACTION_LOCK_SCREEN) })
        }
    }

    /** „Wischen nach unten“: called by `homeGestures` when its detector sees the swipe. */
    fun swipeDown(context: Context) = fire(context, BridgeProtocol.ACTION_NOTIFICATIONS)

    /**
     * Carries out [action] ([BridgeProtocol.ACTION_NOTIFICATIONS] or [BridgeProtocol.ACTION_LOCK_SCREEN]): off without the
     * service (the message once per run); otherwise one `GLOBAL_ACTION`, answered within 2000 ms (a UX limit) or told as unavailable.
     */
    fun fire(context: Context, action: String) {
        if (GestureAvailability.check(context) == Availability.Off) {
            offMessage()
            return
        }
        scope.launch {
            val result = se.globalAction(action)
            when {
                result == null -> se.message(UNAVAILABLE_TEXT)
                result.ok -> Unit
                result.reason == BridgeProtocol.REASON_ACCESSIBILITY_OFF -> offMessage()
                else -> se.message(UNAVAILABLE_TEXT)
            }
        }
    }

    private fun offMessage() {
        if (!offShown.getAndSet(true)) se.message(OFF_TEXT)
    }

    companion object {
        const val OFF_TEXT = "Gesten sind aus. Schalte die Bedienungshilfe in den Einstellungen ein."
        const val UNAVAILABLE_TEXT = "Die Geste ist gerade nicht verfügbar."

        /** Shown once per process run (U40-18). */
        private val offShown = java.util.concurrent.atomic.AtomicBoolean(false)
    }
}
