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
    /** A finger path in display pixels over [durationMs]. */
    data class Stroke(val points: List<Pair<Float, Float>>, val durationMs: Long) : PhoneAction

    /** Text for the focused field; `"\n"` presses Enter. */
    data class Text(val text: String) : PhoneAction

    /** `AccessibilityService.GLOBAL_ACTION_*`. */
    data class Global(val action: Int) : PhoneAction
}

/**
 * Turns the input events of the controlling viewer (the core's `InputEvent`, C5: pointer and touch
 * positions 0–65535 across the source, wheel in 1/120 notches) into gestures, text and global
 * actions of the accessibility service. Key events are left out (`no_key_events`). A pressed
 * button or finger becomes one stroke when it is released. Not thread-safe: the input thread of
 * [ScreenHostControl] owns it.
 */
internal class GestureMapper(private val width: () -> Int, private val height: () -> Int) {
    private var x = 0f
    private var y = 0f
    private var pressed: MutableList<Pair<Float, Float>>? = null
    private var pressedAt = 0L
    private val touches = HashMap<Int, Pair<MutableList<Pair<Float, Float>>, Long>>()

    fun map(events: JsonArray, now: Long = SystemClock.uptimeMillis()): List<PhoneAction> = buildList {
        for (element in events) {
            val event = element as? JsonObject ?: continue
            when (event.text("type")) {
                "pointer_abs" -> {
                    x = scale(event.int("x"), width())
                    y = scale(event.int("y"), height())
                    pressed?.add(x to y)
                }
                "button" -> if (event.text("button") == "left") {
                    if (event.bool("down")) {
                        pressed = mutableListOf(x to y)
                        pressedAt = now
                    } else {
                        pressed?.let { add(PhoneAction.Stroke(it, duration(now - pressedAt))) }
                        pressed = null
                    }
                }
                "wheel" -> {
                    val dy = event.int("dy")
                    if (dy != 0) {
                        // One notch scrolls an eighth of the display; up moves the content down.
                        val distance = height() / 8f * dy / 120f
                        val endY = (y + distance).coerceIn(0f, height() - 1f)
                        add(PhoneAction.Stroke(listOf(x to y, x to endY), 150))
                    }
                }
                "touch" -> {
                    val id = event.int("id")
                    val point = scale(event.int("x"), width()) to scale(event.int("y"), height())
                    when (event.text("phase")) {
                        "start" -> touches[id] = mutableListOf(point) to now
                        "move" -> touches[id]?.first?.add(point)
                        "end" -> touches.remove(id)?.let { (path, since) ->
                            path.add(point)
                            add(PhoneAction.Stroke(path, duration(now - since)))
                        }
                        else -> touches.remove(id)
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
        const val MIN_STROKE_MS = 1L
        const val MAX_STROKE_MS = 10_000L

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
        fun gestureOf(stroke: PhoneAction.Stroke): GestureDescription {
            val path = Path()
            stroke.points.forEachIndexed { index, (px, py) -> if (index == 0) path.moveTo(px, py) else path.lineTo(px, py) }
            // A tap is a path without length: Android needs a point to press.
            if (stroke.points.size == 1) path.lineTo(stroke.points[0].first, stroke.points[0].second)
            return GestureDescription.Builder()
                .addStroke(GestureDescription.StrokeDescription(path, 0, stroke.durationMs))
                .build()
        }

        /** Runs [action] through [service]; `false` when Android refused it. */
        fun perform(service: ScreenControlService, action: PhoneAction, done: (Boolean) -> Unit) {
            when (action) {
                is PhoneAction.Stroke -> service.gesture(gestureOf(action), done)
                is PhoneAction.Text -> done(if (action.text == "\n") service.imeEnter() else service.appendText(action.text))
                is PhoneAction.Global -> done(service.performGlobalAction(action.action))
            }
        }
    }
}
