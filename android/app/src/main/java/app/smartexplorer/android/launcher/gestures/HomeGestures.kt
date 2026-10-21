package app.smartexplorer.android.launcher.gestures

import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp

/**
 * A gesture another package adds to the home screen: [apply] returns the modifier with its detector.
 * U40 adds „Wischen nach unten“ and „Doppeltippen“ here; U35 passes an empty list (U35-29).
 */
fun interface GestureHandler {
    fun apply(modifier: Modifier): Modifier
}

/** Distance of the upward swipe that opens the app list, in dp. */
private const val SWIPE_UP_DP = 48

/**
 * Gestures of the home screen: swipe up opens the app list (U35-29). The swipe counts at the end of the
 * gesture, when the upward travel reached 48 dp or the speed reached four times the system's minimum fling
 * speed. [enabled] is false in edit mode, over a widget (U38) and while a layer is open.
 */
@Composable
fun Modifier.homeGestures(enabled: Boolean, onSwipeUp: () -> Unit, extra: List<GestureHandler> = emptyList()): Modifier {
    val density = LocalDensity.current
    val context = LocalContext.current
    val swipe = rememberUpdatedState(onSwipeUp)
    val distancePx = with(density) { SWIPE_UP_DP.dp.toPx() }
    val speedPx = remember(context) { android.view.ViewConfiguration.get(context).scaledMinimumFlingVelocity * 4f }
    var result = this
    if (enabled) {
        result = result.pointerInput(distancePx, speedPx) {
            var travel = 0f
            val tracker = VelocityTracker()
            detectVerticalDragGestures(
                onDragStart = {
                    travel = 0f
                    tracker.resetTracking()
                },
                onDragEnd = {
                    val speedUp = -tracker.calculateVelocity().y
                    if (-travel >= distancePx || speedUp >= speedPx) swipe.value()
                },
                onDragCancel = { travel = 0f },
                onVerticalDrag = { change, amount ->
                    travel += amount
                    tracker.addPosition(change.uptimeMillis, change.position)
                },
            )
        }
        for (handler in extra) result = handler.apply(result)
    }
    return result
}
