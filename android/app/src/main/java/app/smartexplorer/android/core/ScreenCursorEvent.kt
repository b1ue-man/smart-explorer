package app.smartexplorer.android.core

import java.util.Base64
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull

/**
 * The pointer of a viewer session's host, which travels beside the picture (plan 2026-11-14,
 * package P6): the `screenView` events `cursorShape` and `cursorPosition`. Only a host that
 * can leave the pointer out of the picture sends them; for every other host the pointer stays
 * in the picture and no such event arrives. They are no [ScreenViewEvent]: the session state
 * does not keep them, [app.smartexplorer.android.ui.screen.ScreenCursorOverlay] draws them.
 */
sealed interface ScreenCursorEvent : CoreEvent {
    val session: Long

    /**
     * The pointer's image: [rgba] holds [width] x [height] premultiplied RGBA pixels, [hotX] and
     * [hotY] name the hot spot in them, and a pointer that was too large is thinned out and drawn
     * [shrink] times larger.
     */
    class Shape(
        override val session: Long,
        val id: Long,
        val width: Int,
        val height: Int,
        val hotX: Int,
        val hotY: Int,
        val shrink: Int,
        val rgba: ByteArray,
    ) : ScreenCursorEvent

    /** The hot spot at ([x], [y]) in pixels of the captured area ([areaWidth] x [areaHeight]). */
    data class Position(
        override val session: Long,
        val shapeId: Long,
        val visible: Boolean,
        val x: Int,
        val y: Int,
        val areaWidth: Int,
        val areaHeight: Int,
    ) : ScreenCursorEvent
}

/** The cursor events of the bridge's `screenView` events; `null` for every other kind. */
internal fun decodeScreenCursor(event: JsonObject): ScreenCursorEvent? {
    val session = (event["session"] as? JsonPrimitive)?.longOrNull ?: return null
    fun int(key: String) = (event[key] as? JsonPrimitive)?.intOrNull
    fun long(key: String) = (event[key] as? JsonPrimitive)?.longOrNull
    return when ((event["kind"] as? JsonPrimitive)?.contentOrNull) {
        "cursorShape" -> {
            val width = int("width") ?: return null
            val height = int("height") ?: return null
            val shrink = int("shrink") ?: return null
            val encoded = (event["rgbaBase64"] as? JsonPrimitive)?.contentOrNull ?: return null
            val rgba = try {
                Base64.getDecoder().decode(encoded)
            } catch (e: IllegalArgumentException) {
                return null
            }
            if (width <= 0 || height <= 0 || shrink <= 0 || rgba.size.toLong() != width.toLong() * height * 4) return null
            ScreenCursorEvent.Shape(session, long("id") ?: return null, width, height, int("hotX") ?: return null, int("hotY") ?: return null, shrink, rgba)
        }
        "cursorPosition" -> {
            val areaWidth = int("areaWidth") ?: return null
            val areaHeight = int("areaHeight") ?: return null
            if (areaWidth <= 0 || areaHeight <= 0) return null
            ScreenCursorEvent.Position(
                session,
                long("shapeId") ?: return null,
                (event["visible"] as? JsonPrimitive)?.booleanOrNull ?: false,
                int("x") ?: return null,
                int("y") ?: return null,
                areaWidth,
                areaHeight,
            )
        }
        else -> null
    }
}
