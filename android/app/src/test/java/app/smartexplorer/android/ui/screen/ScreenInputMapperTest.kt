package app.smartexplorer.android.ui.screen

import android.view.KeyEvent
import app.smartexplorer.android.api.ScreenInputCaps
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The mapping of touches and keys to the core's input events (SCR-M9, U28-8) with the times fed in:
 * the signals `screen_task_android_input_touch_to_pointer` and
 * `screen_task_android_input_releases_on_pause`.
 */
class ScreenInputMapperTest {
    private val config = InputConfig(longPressMs = 400, touchSlop = 8f, scrollFactor = 240f)
    private val allCaps = ScreenInputCaps(pointer = true, keyboard = true, text = true, touch = true)

    /** A view position ×10 is the source position, so the expected numbers are easy to read. */
    private fun mapper(host: InputHost, caps: ScreenInputCaps = allCaps, zooms: MutableList<Triple<Float, Float, Float>>? = null) =
        ScreenInputMapper(host, config, { x, y -> (x * 10).toInt() to (y * 10).toInt() }, caps) { scale, fx, fy ->
            zooms?.add(Triple(scale, fx, fy))
        }

    private fun ScreenInputMapper.events() = drain().map { it.toString() }

    private fun pointer(x: Int, y: Int) = """{"type":"pointer_abs","x":$x,"y":$y}"""

    private fun button(name: String, down: Boolean) = """{"type":"button","button":"$name","down":$down}"""

    private fun finger(id: Int, x: Float, y: Float) = Finger(id, x, y)

    @Test
    fun screen_task_android_input_touch_to_pointer() {
        // Tap: the pointer goes to the touch, then a left click; a second tap clicks again.
        val tap = mapper(InputHost.DESKTOP)
        tap.onTouch(TouchKind.DOWN, 0, listOf(finger(0, 100f, 50f)))
        assertTrue("nothing before the lift", tap.events().isEmpty())
        tap.onTouch(TouchKind.UP, 100, listOf(finger(0, 100f, 50f)))
        tap.onTouch(TouchKind.DOWN, 200, listOf(finger(0, 100f, 50f)))
        tap.onTouch(TouchKind.UP, 260, listOf(finger(0, 100f, 50f)))
        val click = listOf(pointer(1000, 500), button("left", true), button("left", false))
        assertEquals(click + click, tap.events())

        // Long press by time alone, without any other event: a right click, once.
        val held = mapper(InputHost.DESKTOP)
        held.onTouch(TouchKind.DOWN, 0, listOf(finger(0, 20f, 30f)))
        held.onTick(399)
        assertTrue(held.events().isEmpty())
        held.onTick(400)
        held.onTouch(TouchKind.UP, 500, listOf(finger(0, 20f, 30f)))
        assertEquals(listOf(pointer(200, 300), button("right", true), button("right", false)), held.events())
        // Long press found at the lift.
        val lifted = mapper(InputHost.DESKTOP)
        lifted.onTouch(TouchKind.DOWN, 0, listOf(finger(0, 20f, 30f)))
        lifted.onTouch(TouchKind.UP, 450, listOf(finger(0, 20f, 30f)))
        assertEquals(listOf(pointer(200, 300), button("right", true), button("right", false)), lifted.events())
        // Movement inside the touch slop keeps it a tap.
        val still = mapper(InputHost.DESKTOP)
        still.onTouch(TouchKind.DOWN, 0, listOf(finger(0, 100f, 100f)))
        still.onTouch(TouchKind.MOVE, 50, listOf(finger(0, 104f, 103f)))
        still.onTouch(TouchKind.UP, 100, listOf(finger(0, 104f, 103f)))
        assertEquals(listOf(pointer(1000, 1000), button("left", true), button("left", false)), still.events())

        // Drag: past the slop the left button goes down at the start, the pointer follows, the lift releases.
        val drag = mapper(InputHost.DESKTOP)
        drag.onTouch(TouchKind.DOWN, 0, listOf(finger(0, 100f, 100f)))
        drag.onTouch(TouchKind.MOVE, 20, listOf(finger(0, 120f, 100f)))
        drag.onTouch(TouchKind.MOVE, 40, listOf(finger(0, 150f, 110f)))
        drag.onTouch(TouchKind.UP, 60, listOf(finger(0, 150f, 110f)))
        assertEquals(
            listOf(pointer(1000, 1000), button("left", true), pointer(1500, 1100), button("left", false)),
            drag.events(),
        )

        // Two fingers moving together roll the wheel; swiping up scrolls down, swiping left scrolls right.
        val scroll = mapper(InputHost.DESKTOP)
        scroll.onTouch(TouchKind.DOWN, 0, listOf(finger(0, 100f, 100f)))
        scroll.onTouch(TouchKind.POINTER_DOWN, 10, listOf(finger(0, 100f, 100f), finger(1, 200f, 100f)), changedId = 1)
        scroll.onTouch(TouchKind.MOVE, 20, listOf(finger(0, 100f, 40f), finger(1, 200f, 40f)))
        assertEquals(listOf("""{"type":"wheel","dx":0,"dy":-30}"""), scroll.events())
        scroll.onTouch(TouchKind.MOVE, 30, listOf(finger(0, 40f, 40f), finger(1, 140f, 40f)))
        assertEquals(listOf("""{"type":"wheel","dx":30,"dy":0}"""), scroll.events())
        scroll.onTouch(TouchKind.POINTER_UP, 40, listOf(finger(0, 40f, 40f), finger(1, 140f, 40f)), changedId = 1)
        scroll.onTouch(TouchKind.UP, 50, listOf(finger(0, 40f, 40f)))
        assertTrue("no click after a scroll", scroll.events().isEmpty())

        // Two fingers pinching send no input and zoom the view around their middle.
        val zooms = mutableListOf<Triple<Float, Float, Float>>()
        val pinch = mapper(InputHost.DESKTOP, zooms = zooms)
        pinch.onTouch(TouchKind.DOWN, 0, listOf(finger(0, 100f, 100f)))
        pinch.onTouch(TouchKind.POINTER_DOWN, 10, listOf(finger(0, 100f, 100f), finger(1, 200f, 100f)), changedId = 1)
        pinch.onTouch(TouchKind.MOVE, 20, listOf(finger(0, 80f, 100f), finger(1, 220f, 100f)))
        pinch.onTouch(TouchKind.UP, 30, listOf(finger(0, 80f, 100f)))
        assertTrue(pinch.events().isEmpty())
        assertEquals(listOf(Triple(1.4f, 150f, 100f)), zooms)

        // The pointer never moves without a touch, and a host without a pointer gets nothing.
        val none = mapper(InputHost.DESKTOP, allCaps.copy(pointer = false))
        none.onTouch(TouchKind.DOWN, 0, listOf(finger(0, 1f, 1f)))
        none.onTouch(TouchKind.UP, 10, listOf(finger(0, 1f, 1f)))
        assertTrue(none.events().isEmpty())
    }

    @Test
    fun screen_task_android_input_touch_goes_to_android_host_as_touch() {
        val m = mapper(InputHost.ANDROID)
        m.onTouch(TouchKind.DOWN, 0, listOf(finger(5, 10f, 20f)), changedId = 5)
        m.onTouch(TouchKind.POINTER_DOWN, 5, listOf(finger(5, 10f, 20f), finger(9, 30f, 40f)), changedId = 9)
        // Two moves in one frame shrink to the last position of each finger.
        m.onTouch(TouchKind.MOVE, 10, listOf(finger(5, 11f, 20f), finger(9, 30f, 41f)))
        m.onTouch(TouchKind.MOVE, 12, listOf(finger(5, 12f, 20f), finger(9, 30f, 42f)))
        m.onTouch(TouchKind.POINTER_UP, 20, listOf(finger(5, 12f, 20f), finger(9, 30f, 42f)), changedId = 9)
        m.onTouch(TouchKind.UP, 30, listOf(finger(5, 12f, 20f)), changedId = 5)
        fun touch(id: Int, phase: String, x: Int, y: Int) = """{"type":"touch","id":$id,"phase":"$phase","x":$x,"y":$y}"""
        assertEquals(
            listOf(
                touch(0, "start", 100, 200),
                touch(1, "start", 300, 400),
                touch(0, "move", 120, 200),
                touch(1, "move", 300, 420),
                touch(1, "end", 300, 420),
                touch(0, "end", 120, 200),
            ),
            m.events(),
        )
        // At most ten fingers.
        val many = mapper(InputHost.ANDROID)
        val down = (0 until 12).map { finger(it, it * 10f, 0f) }
        down.forEachIndexed { index, f -> many.onTouch(TouchKind.POINTER_DOWN, index.toLong(), down.take(index + 1), changedId = f.id) }
        assertEquals(10, many.events().size)
    }

    @Test
    fun screen_task_android_input_releases_on_pause() {
        // A drag with the left button and a held key: pausing answers both, once.
        val m = mapper(InputHost.DESKTOP)
        m.onTouch(TouchKind.DOWN, 0, listOf(finger(0, 100f, 100f)))
        m.onTouch(TouchKind.MOVE, 20, listOf(finger(0, 150f, 100f)))
        m.onKey(KeyEvent.KEYCODE_A, true)
        val released = m.releaseAll().map { it.toString() }
        assertEquals(
            listOf(
                pointer(1000, 1000),
                button("left", true),
                pointer(1500, 1000),
                """{"type":"key","key":"a","down":true}""",
                button("left", false),
                """{"type":"key","key":"a","down":false}""",
            ),
            released,
        )
        assertTrue(m.releaseAll().isEmpty())
        // The lift that follows the pause is not a click.
        m.onTouch(TouchKind.UP, 100, listOf(finger(0, 150f, 100f)))
        assertTrue(m.events().isEmpty())

        // Fingers on an Android host are cancelled at their last position.
        val phone = mapper(InputHost.ANDROID)
        phone.onTouch(TouchKind.DOWN, 0, listOf(finger(1, 10f, 10f)), changedId = 1)
        phone.onTouch(TouchKind.POINTER_DOWN, 1, listOf(finger(1, 10f, 10f), finger(2, 20f, 20f)), changedId = 2)
        phone.drain()
        assertEquals(
            listOf(
                """{"type":"touch","id":0,"phase":"cancel","x":100,"y":100}""",
                """{"type":"touch","id":1,"phase":"cancel","x":200,"y":200}""",
            ),
            phone.releaseAll().map { it.toString() },
        )
        assertTrue(phone.releaseAll().isEmpty())
    }

    @Test
    fun keys_have_the_names_of_the_core() {
        assertEquals("a", ScreenInputMapper.keyName(KeyEvent.KEYCODE_A))
        assertEquals("z", ScreenInputMapper.keyName(KeyEvent.KEYCODE_Z))
        assertEquals("0", ScreenInputMapper.keyName(KeyEvent.KEYCODE_0))
        assertEquals("9", ScreenInputMapper.keyName(KeyEvent.KEYCODE_9))
        assertEquals("f1", ScreenInputMapper.keyName(KeyEvent.KEYCODE_F1))
        assertEquals("f12", ScreenInputMapper.keyName(KeyEvent.KEYCODE_F12))
        assertEquals("backspace", ScreenInputMapper.keyName(KeyEvent.KEYCODE_DEL))
        assertEquals("delete", ScreenInputMapper.keyName(KeyEvent.KEYCODE_FORWARD_DEL))
        assertEquals("home", ScreenInputMapper.keyName(KeyEvent.KEYCODE_MOVE_HOME))
        assertEquals("page_down", ScreenInputMapper.keyName(KeyEvent.KEYCODE_PAGE_DOWN))
        assertEquals("equal", ScreenInputMapper.keyName(KeyEvent.KEYCODE_EQUALS))
        assertEquals("quote", ScreenInputMapper.keyName(KeyEvent.KEYCODE_APOSTROPHE))
        assertEquals("ctrl_left", ScreenInputMapper.keyName(KeyEvent.KEYCODE_CTRL_LEFT))
        assertNull(ScreenInputMapper.keyName(KeyEvent.KEYCODE_VOLUME_UP))
    }

    @Test
    fun modifiers_follow_the_meta_state_and_text_is_cut_at_4096_bytes() {
        val m = mapper(InputHost.DESKTOP)
        m.onKey(KeyEvent.KEYCODE_C, true, KeyEvent.META_CTRL_ON)
        m.onKey(KeyEvent.KEYCODE_C, false, 0)
        assertEquals(
            listOf(
                """{"type":"key","key":"ctrl_left","down":true}""",
                """{"type":"key","key":"c","down":true}""",
                """{"type":"key","key":"c","down":false}""",
                """{"type":"key","key":"ctrl_left","down":false}""",
            ),
            m.events(),
        )
        m.onText("a".repeat(5000))
        val text = m.drain()
        assertEquals(2, text.size)
        assertEquals(4096 + 904, text.sumOf { it["text"].toString().trim('"').length })
        // A host that takes no text gets none.
        val silent = mapper(InputHost.DESKTOP, allCaps.copy(text = false))
        silent.onText("hallo")
        assertTrue(silent.events().isEmpty())
        // Toolbar key and action.
        m.tapUsage(0xE3)
        m.action("home")
        assertEquals(
            listOf(
                """{"type":"key","hid_usage":227,"down":true}""",
                """{"type":"key","hid_usage":227,"down":false}""",
                """{"type":"action","action":"home"}""",
            ),
            m.events(),
        )
    }

    /** The hidden keyboard field: appends, autocorrect and deletions become backspaces plus text; padding is never typed. */
    @Test
    fun screen_task_android_keyboard_field_edits_become_text() {
        val pad = "\u200B".repeat(4)
        assertEquals(0 to "a", keyboardEdit(pad, pad + "a"))
        assertEquals(0 to "a", keyboardEdit(pad, "a" + pad))
        assertEquals(0 to "lo", keyboardEdit(pad + "hel", pad + "hello"))
        assertEquals(2 to "he ", keyboardEdit(pad + "teh", pad + "the "))
        assertEquals(1 to "", keyboardEdit(pad + "ab", pad + "a"))
        assertEquals(1 to "", keyboardEdit(pad, pad.dropLast(1)))
        assertEquals(0 to "ä", keyboardEdit(pad, pad + "ä"))
    }
}
