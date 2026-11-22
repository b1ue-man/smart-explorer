package app.smartexplorer.android.screen

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.graphics.Path
import android.os.SystemClock
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull

/** One action on the phone that the input of a viewer becomes. */
internal sealed interface PhoneAction {
    /** A finger path in display pixels over [durationMs], beginning [startMs] after its gesture began. */
    data class Stroke(val points: List<Pair<Float, Float>>, val durationMs: Long, val startMs: Long = 0) : PhoneAction

    /** Fingers that touched at the same time (a pinch): one gesture of several [strokes] (Codex 4/23). */
    data class Gesture(val strokes: List<Stroke>) : PhoneAction

    /** Text for the focused field; `"\n"` presses Enter. */
    data class Text(val text: String) : PhoneAction

    /** `AccessibilityService.GLOBAL_ACTION_*`. */
    data class Global(val action: Int) : PhoneAction

    /** An input Android has no counterpart for ([kind] for the viewer's notice): nothing is performed (Codex 9/14). */
    data class Refused(val kind: String) : PhoneAction
}

/**
 * Turns the input events of the controlling viewer (the core's `InputEvent`, C5: pointer and touch
 * positions 0–65535 across the source, wheel in 1/120 notches) into gestures, text and global
 * actions of the accessibility service ([AccessLink.perform]). Key events are left out (`no_key_events`). A pressed
 * button or finger becomes one stroke when it is released. Not thread-safe: the input thread of
 * [ScreenHostControl] owns it.
 */
internal class GestureMapper(private val width: () -> Int, private val height: () -> Int) {
    private var x = 0f
    private var y = 0f
    /** A pointer position arrived: before it, the wheel scrolls from the middle of the display, not from (0, 0). */
    private var positioned = false
    /** The parts of a wheel notch (1/120) per axis not turned into a stroke yet: one stroke per whole notch. */
    private var wheelRestX = 0
    private var wheelRestY = 0
    private var pressed: MutableList<Pair<Float, Float>>? = null
    private var pressedAt = 0L
    private val touches = HashMap<Int, Pair<MutableList<Pair<Float, Float>>, Long>>()
    /** The ended strokes of fingers that overlapped others still down; they go out together with the last. */
    private val group = mutableListOf<PhoneAction.Stroke>()
    private var groupStart = 0L

    /** The fingers are all up: their strokes go out as one gesture (one finger: one stroke, as before). */
    private fun flushGroup(out: MutableList<PhoneAction>) {
        when (group.size) {
            0 -> Unit
            1 -> out.add(group[0].copy(startMs = 0))
            else -> out.add(PhoneAction.Gesture(group.take(MAX_STROKES)))
        }
        group.clear()
    }

    fun map(events: JsonArray, now: Long = SystemClock.uptimeMillis()): List<PhoneAction> = buildList {
        for (element in events) {
            val event = element as? JsonObject ?: continue
            when (event.text("type")) {
                "pointer_abs" -> {
                    x = scale(event.int("x"), width())
                    y = scale(event.int("y"), height())
                    positioned = true
                    pressed?.add(x to y)
                }
                "button" -> when (event.text("button")) {
                    "left" -> if (event.bool("down")) {
                        pressed = mutableListOf(x to y)
                        pressedAt = now
                    } else {
                        pressed?.let { add(PhoneAction.Stroke(it, duration(now - pressedAt))) }
                        pressed = null
                    }
                    // The mouse's back button is Android's Back, as with a mouse on the phone itself (Codex 9/14).
                    "back" -> if (!event.bool("down")) add(PhoneAction.Global(AccessibilityService.GLOBAL_ACTION_BACK))
                    // Right, middle and forward have no counterpart an accessibility service can perform: said, not
                    // dropped in silence (once, at the press).
                    else -> if (event.bool("down")) add(PhoneAction.Refused("button"))
                }
                "wheel" -> {
                    // A touchpad sends parts of a notch; each would become a stroke of its own (150 ms, performed one
                    // after another), so the parts add up and only whole notches make a stroke.
                    wheelRestX += event.int("dx")
                    wheelRestY += event.int("dy")
                    val dx = wheelRestX / NOTCH * NOTCH
                    val dy = wheelRestY / NOTCH * NOTCH
                    wheelRestX -= dx
                    wheelRestY -= dy
                    if (dx != 0) {
                        // Right is positive (C5): the content moves left, so the finger does; an eighth of the width per notch.
                        val distance = width() / 8f * dx / 120f
                        val (fromX, fromY) = if (positioned) x to y else (width() - 1).coerceAtLeast(0) / 2f to (height() - 1).coerceAtLeast(0) / 2f
                        val endX = (fromX - distance).coerceIn(0f, (width() - 1).coerceAtLeast(0).toFloat())
                        add(PhoneAction.Stroke(listOf(fromX to fromY, endX to fromY), 150))
                    }
                    if (dy != 0) {
                        // One notch scrolls an eighth of the display; up moves the content down.
                        val distance = height() / 8f * dy / 120f
                        // Without a position (`se screen input --scroll`) a stroke from the top edge would pull down the
                        // notification shade: it starts in the middle of the display instead.
                        val (fromX, fromY) = if (positioned) x to y else (width() - 1).coerceAtLeast(0) / 2f to (height() - 1).coerceAtLeast(0) / 2f
                        val endY = (fromY + distance).coerceIn(0f, (height() - 1).coerceAtLeast(0).toFloat())
                        add(PhoneAction.Stroke(listOf(fromX to fromY, fromX to endY), 150))
                    }
                }
                "touch" -> {
                    val id = event.int("id")
                    val point = scale(event.int("x"), width()) to scale(event.int("y"), height())
                    when (event.text("phase")) {
                        "start" -> {
                            if (touches.isEmpty() && group.isEmpty()) groupStart = now
                            touches[id] = mutableListOf(point) to now
                        }
                        "move" -> touches[id]?.first?.add(point)
                        "end" -> {
                            touches.remove(id)?.let { (path, since) ->
                                path.add(point)
                                group.add(PhoneAction.Stroke(path, duration(now - since), (since - groupStart).coerceIn(0L, MAX_STROKE_MS)))
                            }
                            if (touches.isEmpty()) flushGroup(this)
                        }
                        else -> {
                            touches.remove(id)
                            if (touches.isEmpty()) flushGroup(this)
                        }
                    }
                }
                "text" -> event.text("text")?.takeIf { it.isNotEmpty() }?.let { add(PhoneAction.Text(it)) }
                "action" -> globalOf(event.text("action"))?.let { add(PhoneAction.Global(it)) }
                // "key": Android takes no key events from an accessibility service.
                else -> Unit
            }
        }
    }

    private fun scale(value: Int, side: Int): Float = (value.coerceIn(0, 65_535) / 65_535f) * (side - 1).coerceAtLeast(0)

    private fun duration(ms: Long): Long = ms.coerceIn(MIN_STROKE_MS, MAX_STROKE_MS)

    private fun JsonObject.text(key: String): String? = (this[key] as? JsonPrimitive)?.contentOrNull

    private fun JsonObject.int(key: String): Int = (this[key] as? JsonPrimitive)?.intOrNull ?: 0

    private fun JsonObject.bool(key: String): Boolean = (this[key] as? JsonPrimitive)?.booleanOrNull ?: false

    companion object {
        /** One wheel notch in the core's units (C5: 1/120). */
        private const val NOTCH = 120
        const val MIN_STROKE_MS = 1L
        const val MAX_STROKE_MS = 10_000L
        /** Strokes of one gesture: `GestureDescription.getMaxStrokeCount()` is at least 10. */
        const val MAX_STROKES = 10

        fun globalOf(action: String?): Int? = when (action) {
            "back" -> AccessibilityService.GLOBAL_ACTION_BACK
            "home" -> AccessibilityService.GLOBAL_ACTION_HOME
            "recents" -> AccessibilityService.GLOBAL_ACTION_RECENTS
            "notifications" -> AccessibilityService.GLOBAL_ACTION_NOTIFICATIONS
            "quick_settings" -> AccessibilityService.GLOBAL_ACTION_QUICK_SETTINGS
            "lock_screen" -> AccessibilityService.GLOBAL_ACTION_LOCK_SCREEN
            "power_dialog" -> AccessibilityService.GLOBAL_ACTION_POWER_DIALOG
            else -> null
        }

        /** [stroke] as one gesture of the accessibility service. */
        fun gestureOf(stroke: PhoneAction.Stroke): GestureDescription = gestureOf(listOf(stroke))

        /** [strokes] as one gesture, each from its [PhoneAction.Stroke.startMs] (several fingers at once). */
        fun gestureOf(strokes: List<PhoneAction.Stroke>): GestureDescription {
            val builder = GestureDescription.Builder()
            for (stroke in strokes.take(MAX_STROKES)) {
                val path = Path()
                stroke.points.forEachIndexed { index, (px, py) -> if (index == 0) path.moveTo(px, py) else path.lineTo(px, py) }
                // A tap is a path without length: Android needs a point to press.
                if (stroke.points.size == 1) path.lineTo(stroke.points[0].first, stroke.points[0].second)
                builder.addStroke(GestureDescription.StrokeDescription(path, stroke.startMs, stroke.durationMs))
            }
            return builder.build()
        }
    }
}
