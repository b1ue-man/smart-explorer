package app.smartexplorer.android.ui.screen

import android.os.SystemClock
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.PointerInputChange
import androidx.compose.ui.input.pointer.changedToDown
import androidx.compose.ui.input.pointer.changedToUp
import androidx.compose.ui.input.pointer.pointerInput
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/** Which kind of device the input goes to (`platform` of the host). */
enum class InputHost { DESKTOP, ANDROID }

/** System values of the touch handling (`ViewConfiguration`), injected so a test sets its own. */
data class InputConfig(val longPressMs: Long, val touchSlop: Float, val scrollFactor: Float)

/** One finger in view pixels. */
data class Finger(val id: Int, val x: Float, val y: Float)

/** The action of a `MotionEvent`; for [POINTER_UP] and [UP] the lifted finger is still in the list. */
enum class TouchKind { DOWN, POINTER_DOWN, MOVE, POINTER_UP, UP, CANCEL }

/**
 * Zoom and pan of the picture (SCR-M9, U28-7); [onChange] tells the screen to draw again. [zoom] runs from 1× (fits the view) to [MAX_ZOOM]×;
 * the picture keeps its aspect ratio, is centered, and with zoom > 1 it can be moved until its edge
 * meets the edge of the view. All sizes are view pixels.
 */
class ViewTransform(private val onChange: () -> Unit = {}) {
    var zoom = 1f
        private set
    var panX = 0f
        private set
    var panY = 0f
        private set

    private var nativeSize: Pair<Float, Float>? = null
    fun nativePixels(width: Int?, height: Int?) {
        nativeSize = if (width != null && height != null && width > 0 && height > 0) width.toFloat() to height.toFloat() else null
        reset()
    }

    /** Size of the zoomed picture inside a view of [viewW]×[viewH] for a picture of [aspect] (width/height). */
    fun contentSize(viewW: Float, viewH: Float, aspect: Float): Pair<Float, Float> {
        val native = nativeSize
        if (native != null) return (native.first * zoom).roundToInt().toFloat() to (native.second * zoom).roundToInt().toFloat()
        val fitW = min(viewW, viewH * aspect)
        return (fitW * zoom).roundToInt().coerceAtLeast(1).toFloat() to (fitW / aspect * zoom).roundToInt().coerceAtLeast(1).toFloat()
    }

    /** Zooms by [scale] around the view point ([focusX], [focusY]); the point stays under the finger. */
    fun zoomBy(scale: Float, focusX: Float, focusY: Float, viewW: Float, viewH: Float, aspect: Float) {
        val next = (zoom * scale).coerceIn(1f, MAX_ZOOM)
        val ratio = next / zoom
        val fx = focusX - viewW / 2
        val fy = focusY - viewH / 2
        zoom = next
        panX = (panX - fx) * ratio + fx
        panY = (panY - fy) * ratio + fy
        clamp(viewW, viewH, aspect)
        onChange()
    }

    fun panBy(dx: Float, dy: Float, viewW: Float, viewH: Float, aspect: Float) {
        panX += dx
        panY += dy
        clamp(viewW, viewH, aspect)
        onChange()
    }

    fun reset() {
        zoom = 1f
        panX = 0f
        panY = 0f
        onChange()
    }

    private fun clamp(viewW: Float, viewH: Float, aspect: Float) {
        val (w, h) = contentSize(viewW, viewH, aspect)
        val limitX = max(0f, (w - viewW) / 2)
        val limitY = max(0f, (h - viewH) / 2)
        panX = panX.coerceIn(-limitX, limitX)
        panY = panY.coerceIn(-limitY, limitY)
    }

    /** The position 0–65535 across the source of a view point (clamped to the picture). */
    fun toSource(x: Float, y: Float, viewW: Float, viewH: Float, aspect: Float): Pair<Int, Int> {
        val (w, h) = contentSize(viewW, viewH, aspect)
        val left = ((viewW - w) / 2 + panX).roundToInt().toFloat()
        val top = ((viewH - h) / 2 + panY).roundToInt().toFloat()
        val axis = { value: Float, size: Float -> (value / size * FULL_RANGE).toInt().coerceIn(0, FULL_RANGE.toInt()) }
        return axis(x - left, w) to axis(y - top, h)
    }

    companion object {
        /** Above this a source pixel is larger than 8 screen pixels: a range without use. */
        const val MAX_ZOOM = 8f
        private const val FULL_RANGE = 65_535f
    }
}

/**
 * Hands the touches on the view to [mapper] (SCR-M9, U28-8) and calls [onInput] after each one, so
 * the caller sends the frame's events. A held touch is turned into a long press when its time is
 * over; the changes are consumed so no other detector sees them.
 */
fun Modifier.mapperTouchInput(mapper: ScreenInputMapper, longPressMs: Long, onInput: () -> Unit): Modifier =
    pointerInput(mapper) {
        awaitPointerEventScope {
            val down = HashSet<Long>()
            while (true) {
                val event = if (down.isEmpty()) {
                    awaitPointerEvent()
                } else {
                    withTimeoutOrNull(longPressMs) { awaitPointerEvent() }
                }
                if (event == null) {
                    mapper.onTick(SystemClock.uptimeMillis())
                    onInput()
                    continue
                }
                if (event.changes.any { it.isConsumed }) {
                    mapper.onTouch(TouchKind.CANCEL,SystemClock.uptimeMillis(),emptyList(),-1)
                    down.clear()
                    onInput()
                    continue
                }
                val time = event.changes.maxOf { it.uptimeMillis }
                val fingers = event.changes.filter { it.pressed || it.changedToUp() }
                    .map { Finger(it.id.value.toInt(), it.position.x, it.position.y) }
                val first = event.changes.firstOrNull { it.changedToDown() || it.changedToUp() }
                val kind = when {
                    first != null && first.changedToDown() -> if (down.isEmpty()) TouchKind.DOWN else TouchKind.POINTER_DOWN
                    first != null -> if (down.size <= 1) TouchKind.UP else TouchKind.POINTER_UP
                    else -> TouchKind.MOVE
                }
                event.changes.forEach { change -> track(down, change) }
                mapper.onTouch(kind, time, fingers, first?.id?.value?.toInt() ?: -1)
                event.changes.forEach { it.consume() }
                onInput()
            }
        }
    }

private fun track(down: MutableSet<Long>, change: PointerInputChange) {
    if (change.pressed) down.add(change.id.value) else down.remove(change.id.value)
}
