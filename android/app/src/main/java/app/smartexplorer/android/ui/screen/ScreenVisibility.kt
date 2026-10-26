package app.smartexplorer.android.ui.screen

import android.graphics.Rect as AndroidRect
import android.view.KeyEvent
import android.view.View
import android.view.ViewTreeObserver
import android.view.WindowInsets
import android.view.WindowInsetsController
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.window.DialogWindowProvider
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.math.ceil
import kotlin.math.floor

data class ScreenVisibleRect(val x: Int, val y: Int, val width: Int, val height: Int)
data class ScreenVisibilityReport(val geometryGeneration: Long, val state: String,
    val minimized: Boolean, val rects: List<ScreenVisibleRect>) {
    fun asJson(): JsonObject = buildJsonObject {
        put("geometry_generation", geometryGeneration)
        put("state", state)
        put("minimized", minimized)
        put("rects", buildJsonArray { rects.forEach { r -> add(buildJsonObject {
            put("x",r.x); put("y",r.y); put("width",r.width); put("height",r.height)
        }) } })
    }
}
internal data class ScreenVisibilityFacts(val visible: Boolean, val viewport: Rect,
    val clipped: Rect, val content: Rect, val sourceWidth: Int, val sourceHeight: Int, val sourceKey: String? = null) {
    fun rects(): List<ScreenVisibleRect> {
        val r = clipped.intersect(content)
        if (!visible || r.width <= 0 || r.height <= 0 || content.width <= 0 || content.height <= 0 || sourceWidth <= 0 || sourceHeight <= 0) return emptyList()
        val x = floor((r.left-content.left)/content.width*sourceWidth).toInt().coerceIn(0,sourceWidth)
        val y = floor((r.top-content.top)/content.height*sourceHeight).toInt().coerceIn(0,sourceHeight)
        val right = ceil((r.right-content.left)/content.width*sourceWidth).toInt().coerceIn(x,sourceWidth)
        val bottom = ceil((r.bottom-content.top)/content.height*sourceHeight).toInt().coerceIn(y,sourceHeight)
        return listOf(ScreenVisibleRect(x,y,right-x,bottom-y))
    }
}

/** STARTED, not RESUMED: a paused split-screen activity still displays frames.
 * Focus changes merely resample real bounds, never manufacture occlusion. */
@Composable
internal fun Modifier.screenVisibility(content: Rect, sourceWidth: Int, sourceHeight: Int,
    onVisibility: (ScreenVisibilityFacts) -> Unit): Modifier {
    val view = LocalView.current
    val owner = LocalLifecycleOwner.current
    val callback by rememberUpdatedState(onVisibility)
    val contentNow by rememberUpdatedState(content)
    val widthNow by rememberUpdatedState(sourceWidth)
    val heightNow by rememberUpdatedState(sourceHeight)
    val coordinates = remember { arrayOfNulls<LayoutCoordinates>(1) }
    fun observe() {
        val bounds = coordinates[0]?.takeIf { it.isAttached }?.boundsInWindow() ?: return
        val global = AndroidRect()
        val screen = IntArray(2)
        val window = IntArray(2)
        view.getLocationOnScreen(screen); view.getLocationInWindow(window)
        val visible = owner.lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED) && view.isShown &&
            view.windowVisibility == View.VISIBLE && view.getGlobalVisibleRect(global)
        val clipWindow = Rect((global.left-screen[0]+window[0]).toFloat(),(global.top-screen[1]+window[1]).toFloat(),
            (global.right-screen[0]+window[0]).toFloat(),(global.bottom-screen[1]+window[1]).toFloat()).intersect(bounds)
        val clip = clipWindow.translate(-bounds.left,-bounds.top)
        callback(ScreenVisibilityFacts(visible && clip.width > 0 && clip.height > 0,
            Rect(0f,0f,bounds.width,bounds.height),clip,contentNow,widthNow,heightNow))
    }
    SideEffect { observe() }
    DisposableEffect(view,owner) {
        val lifecycle = LifecycleEventObserver { _, _ -> observe() }
        val layout = ViewTreeObserver.OnGlobalLayoutListener { observe() }
        val focus = ViewTreeObserver.OnWindowFocusChangeListener { observe() }
        owner.lifecycle.addObserver(lifecycle)
        view.viewTreeObserver.addOnGlobalLayoutListener(layout)
        view.viewTreeObserver.addOnWindowFocusChangeListener(focus)
        onDispose {
            owner.lifecycle.removeObserver(lifecycle)
            if (view.viewTreeObserver.isAlive) {
                view.viewTreeObserver.removeOnGlobalLayoutListener(layout)
                view.viewTreeObserver.removeOnWindowFocusChangeListener(focus)
            }
        }
    }
    return onGloballyPositioned { coordinates[0] = it; observe() }
}

internal fun localFullscreenKey(event: KeyEvent): Boolean = event.keyCode == KeyEvent.KEYCODE_F11 ||
    (event.isCtrlPressed && event.isAltPressed && (event.keyCode == KeyEvent.KEYCODE_BREAK ||
        event.keyCode == KeyEvent.KEYCODE_F && event.isShiftPressed))

/** Uses the dialog's actual window; system edge swipes remain available. */
@Composable
internal fun ScreenImmersive(enabled: Boolean) {
    val view = LocalView.current
    DisposableEffect(view,enabled) {
        val window = (view.parent as? DialogWindowProvider)?.window
        val controller = window?.insetsController
        val previous = controller?.systemBarsBehavior
        val wasStatus = view.rootWindowInsets?.isVisible(WindowInsets.Type.statusBars()) ?: true
        val wasNavigation = view.rootWindowInsets?.isVisible(WindowInsets.Type.navigationBars()) ?: true
        if (enabled) {
            controller?.systemBarsBehavior = WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            controller?.hide(WindowInsets.Type.systemBars())
        } else controller?.show(WindowInsets.Type.systemBars())
        onDispose {
            if (previous != null) controller?.systemBarsBehavior = previous
            if (wasStatus) controller?.show(WindowInsets.Type.statusBars()) else controller?.hide(WindowInsets.Type.statusBars())
            if (wasNavigation) controller?.show(WindowInsets.Type.navigationBars()) else controller?.hide(WindowInsets.Type.navigationBars())
        }
    }
}
