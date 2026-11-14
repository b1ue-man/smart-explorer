package app.smartexplorer.android.core

import app.smartexplorer.android.ui.screen.cursorRect
import java.util.Base64
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The pointer overlay's events (plan 2026-11-14, package P6) through the app's own decoder. */
class ScreenCursorEventTest {
    private fun shapeJson(pixels: Int) =
        """{"type":"screenView","session":7,"kind":"cursorShape","id":4,"width":$pixels,"height":1,"hotX":1,"hotY":0,"shrink":2,
            "rgbaBase64":"${Base64.getEncoder().encodeToString(ByteArray(pixels * 4) { 9 })}"}"""

    @Test
    fun cursorEventsDecodeBesideTheSessionEvents() {
        val raw = """{"ok":[
            ${shapeJson(2)},
            {"type":"screenView","session":7,"kind":"cursorPosition","shapeId":4,"visible":true,"x":400,"y":300,"areaWidth":800,"areaHeight":600},
            {"type":"screenView","session":7,"kind":"state","state":"streaming"}
        ]}"""
        val events = CoreEvents.parse(Core.json, raw)!!
        assertEquals(3, events.size)
        val shape = events[0] as ScreenCursorEvent.Shape
        assertEquals(2, shape.width)
        assertEquals(8, shape.rgba.size)
        assertEquals(ScreenCursorEvent.Position(7, 4, true, 400, 300, 800, 600), events[1])
        assertTrue("the state event keeps its own type", events[2] is ScreenViewEvent.State)
    }

    @Test
    fun aShapeWithTheWrongSizeOrNoPixelsIsSkipped() {
        assertNull(decodeScreenCursor(kotlinx.serialization.json.Json.parseToJsonElement(shapeJson(2).replace("\"width\":2", "\"width\":3")) as kotlinx.serialization.json.JsonObject))
        assertNull(decodeScreenCursor(kotlinx.serialization.json.Json.parseToJsonElement("""{"session":7,"kind":"cursorPosition","shapeId":4,"x":1,"y":2,"areaWidth":0,"areaHeight":5}""") as kotlinx.serialization.json.JsonObject))
    }

    @Test
    fun theHostsNothingToDrawDecodesAsAHiddenPosition() {
        // What a host sends first after a request and what a new connection starts with.
        val raw = """{"ok":[{"type":"screenView","session":7,"kind":"cursorPosition","shapeId":0,"visible":false,"x":0,"y":0,"areaWidth":1,"areaHeight":1}]}"""
        assertEquals(listOf<CoreEvent>(ScreenCursorEvent.Position(7, 0, false, 0, 0, 1, 1)), CoreEvents.parse(Core.json, raw))
    }

    @Test
    fun theHotSpotLiesOnTheHostPositionScaledToThePicture() {
        val shape = ScreenCursorEvent.Shape(7, 4, 8, 12, 2, 3, 1, ByteArray(8 * 12 * 4))
        val position = ScreenCursorEvent.Position(7, 4, true, 400, 300, 800, 600)
        // The picture lies at (20, 10) and is 200 x 150 pixels: a quarter of the captured area.
        assertEquals(listOf(120, 84, 2, 3), cursorRect(shape, position, 20f, 10f, 200f, 150f))
    }
}
