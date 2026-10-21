package app.smartexplorer.android.ui.screen

import android.view.KeyEvent
import app.smartexplorer.android.api.ScreenInputCaps
import kotlin.math.abs
import kotlin.math.hypot
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * Touches and keys as the core's input events (SCR-M9, U28-8) while „Steuern“ is on; [drain] hands
 * over the events of one frame, times come from the caller. Desktop host: tap = left click, held
 * without moving = right click, one finger past the touch slop = drag with the left button, two
 * fingers moving together = wheel (swiping up scrolls down), pinching = [onZoom] and no input.
 * Android host: every finger (at most [MAX_FINGERS]) is a `touch` event.
 */
class ScreenInputMapper(
    private val host: InputHost,
    private val config: InputConfig,
    /** The position 0–65535 across the source of a view position. */
    private val toSource: (Float, Float) -> Pair<Int, Int>,
    private val caps: ScreenInputCaps = ScreenInputCaps(pointer = true, keyboard = true, text = true, touch = true),
    private val onZoom: (scale: Float, focusX: Float, focusY: Float) -> Unit = { _, _, _ -> },
) {
    private enum class Mode { IDLE, PENDING, DRAG, LONG_DONE, TWO, SCROLL, ZOOM }

    private val queue = ArrayList<JsonObject>()
    private val heldButtons = LinkedHashSet<String>()
    private val heldKeys = LinkedHashSet<String>()
    private val slots = HashMap<Int, Int>()
    private val slotPositions = HashMap<Int, Pair<Int, Int>>()
    private var mode = Mode.IDLE
    private var downTime = 0L
    private var start = 0f to 0f
    private var cur = 0f to 0f
    private var startMid = 0f to 0f
    private var lastMid = 0f to 0f
    private var startDistance = 0f
    private var lastDistance = 0f
    private var scroll = 0f to 0f

    /** One touch action; [fingers] are all fingers at that moment, [changedId] the one that went down or up. */
    fun onTouch(kind: TouchKind, timeMs: Long, fingers: List<Finger>, changedId: Int = -1) {
        if (host == InputHost.ANDROID) androidTouch(kind, fingers, changedId) else desktopTouch(kind, timeMs, fingers)
    }

    /** A held touch becomes a long press once its time is over, without a new touch event. */
    fun onTick(timeMs: Long) {
        if (host == InputHost.DESKTOP && mode == Mode.PENDING && timeMs - downTime >= config.longPressMs) longPress()
    }

    private fun longPress() = click("right", start).also { mode = Mode.LONG_DONE }

    private fun desktopTouch(kind: TouchKind, timeMs: Long, fingers: List<Finger>) {
        val first = fingers.firstOrNull()
        if (!caps.pointer || first == null && kind != TouchKind.CANCEL) return
        when (kind) {
            TouchKind.DOWN -> {
                mode = Mode.PENDING
                downTime = timeMs
                start = first!!.x to first.y
                cur = start
            }
            TouchKind.POINTER_DOWN ->
                if (fingers.size >= 2 && (mode == Mode.PENDING || mode == Mode.TWO)) beginTwo(fingers[0], fingers[1])
            TouchKind.MOVE -> desktopMove(timeMs, fingers)
            TouchKind.UP, TouchKind.CANCEL, TouchKind.POINTER_UP -> {
                if (kind == TouchKind.POINTER_UP) return
                if (mode == Mode.PENDING && kind == TouchKind.UP) {
                    click(if (timeMs - downTime >= config.longPressMs) "right" else "left", start)
                } else if (mode == Mode.DRAG) {
                    if (kind == TouchKind.UP) add(pointer(cur))
                    button("left", false)
                }
                mode = Mode.IDLE
            }
        }
    }

    private fun desktopMove(timeMs: Long, fingers: List<Finger>) {
        if (mode == Mode.TWO || mode == Mode.SCROLL || mode == Mode.ZOOM) {
            if (fingers.size >= 2) twoMove(fingers[0], fingers[1])
            return
        }
        if (mode != Mode.PENDING && mode != Mode.DRAG) return
        cur = fingers.first().let { it.x to it.y }
        if (mode == Mode.PENDING) {
            if (hypot(cur.first - start.first, cur.second - start.second) <= config.touchSlop) {
                if (timeMs - downTime >= config.longPressMs) longPress()
                return
            }
            add(pointer(start))
            button("left", true)
            mode = Mode.DRAG
        }
        add(pointer(cur))
    }

    private fun beginTwo(a: Finger, b: Finger) {
        mode = Mode.TWO
        startDistance = hypot(a.x - b.x, a.y - b.y).also { lastDistance = it }
        startMid = (a.x + b.x) / 2 to (a.y + b.y) / 2
        lastMid = startMid
        scroll = 0f to 0f
    }

    private fun twoMove(a: Finger, b: Finger) {
        val mid = (a.x + b.x) / 2 to (a.y + b.y) / 2
        val distance = hypot(a.x - b.x, a.y - b.y)
        if (mode == Mode.TWO) {
            val spread = abs(distance - startDistance)
            val shift = hypot(mid.first - startMid.first, mid.second - startMid.second)
            if (maxOf(spread, shift) <= config.touchSlop) return
            mode = if (spread > shift) Mode.ZOOM else Mode.SCROLL
        }
        if (mode == Mode.ZOOM) {
            if (lastDistance > 0f && distance > 0f) onZoom(distance / lastDistance, mid.first, mid.second)
            lastDistance = distance
        } else {
            scroll = scroll.first + mid.first - lastMid.first to scroll.second + mid.second - lastMid.second
            val unit = config.scrollFactor / WHEEL_NOTCH
            val ux = (scroll.first / unit).toInt()
            val uy = (scroll.second / unit).toInt()
            if (ux != 0 || uy != 0) {
                scroll = scroll.first - ux * unit to scroll.second - uy * unit
                // The content follows the fingers: swiping up scrolls down, swiping left scrolls right.
                add(ev("wheel", "dx" to (-ux).coerceIn(-32768, 32767), "dy" to uy.coerceIn(-32768, 32767)))
            }
        }
        lastMid = mid
    }

    private fun androidTouch(kind: TouchKind, fingers: List<Finger>, changedId: Int) {
        if (!caps.touch) return
        val changed = fingers.firstOrNull { it.id == changedId }
        when (kind) {
            TouchKind.DOWN, TouchKind.POINTER_DOWN -> if (changed != null) {
                val slot = (0 until MAX_FINGERS).firstOrNull { it !in slots.values } ?: return
                slots[changed.id] = slot
                touch(slot, "start", changed)
            }
            TouchKind.MOVE -> fingers.forEach { finger ->
                val slot = slots[finger.id] ?: return@forEach
                if (toSource(finger.x, finger.y) != slotPositions[slot]) touch(slot, "move", finger)
            }
            TouchKind.POINTER_UP, TouchKind.UP -> if (changed != null) {
                val slot = slots.remove(changed.id) ?: return
                touch(slot, "end", changed)
                slotPositions.remove(slot)
            }
            TouchKind.CANCEL -> cancelTouches()
        }
    }

    private fun touch(slot: Int, phase: String, finger: Finger) {
        val (x, y) = toSource(finger.x, finger.y)
        slotPositions[slot] = x to y
        add(ev("touch", "id" to slot, "phase" to phase, "x" to x, "y" to y))
    }

    private fun cancelTouches() {
        slots.values.sorted().forEach { slot ->
            val (x, y) = slotPositions[slot] ?: (0 to 0)
            add(ev("touch", "id" to slot, "phase" to "cancel", "x" to x, "y" to y))
        }
        slots.clear()
        slotPositions.clear()
    }

    /** A key of the hardware keyboard; [metaState] (`KeyEvent.metaState`) brings the modifiers. */
    fun onKey(keyCode: Int, down: Boolean, metaState: Int = 0) {
        val name = keyName(keyCode)
        if (!caps.keyboard || name == null) return
        if (name !in MODIFIERS && down) syncModifiers(metaState)
        key(name, down)
        if (name !in MODIFIERS && !down) syncModifiers(metaState)
    }

    /** Text typed on the screen keyboard; one event holds at most 4096 bytes, longer text travels as several. */
    fun onText(text: String) {
        if (!caps.text || text.isEmpty()) return
        var chunk = ""
        for (piece in text.codePoints().toArray().map { String(Character.toChars(it)) }) {
            if ((chunk + piece).toByteArray().size > MAX_TEXT_BYTES) {
                add(ev("text", "text" to chunk))
                chunk = ""
            }
            chunk += piece
        }
        add(ev("text", "text" to chunk))
    }

    fun tapKey(name: String) {
        if (caps.keyboard) listOf(true, false).forEach { key(name, it) }
    }

    /** A key of the host's toolbar by its HID usage, pressed and released. */
    fun tapUsage(usage: Int) {
        if (caps.keyboard) listOf(true, false).forEach { add(ev("key", "hid_usage" to usage, "down" to it)) }
    }

    /** A system action of an Android host (`back`, `home`, …). */
    fun action(name: String) = add(ev("action", "action" to name))

    private fun key(name: String, down: Boolean) {
        if (down) heldKeys.add(name) else heldKeys.remove(name)
        add(ev("key", "key" to name, "down" to down))
    }

    private fun syncModifiers(meta: Int) {
        for ((bit, left, right) in listOf(
            Triple(KeyEvent.META_CTRL_ON, "ctrl_left", "ctrl_right"),
            Triple(KeyEvent.META_SHIFT_ON, "shift_left", "shift_right"),
            Triple(KeyEvent.META_ALT_ON, "alt_left", "alt_right"),
        )) {
            val held = listOf(left, right).filter { it in heldKeys }
            if (meta and bit != 0 && held.isEmpty()) key(left, true)
            if (meta and bit == 0) held.forEach { key(it, false) }
        }
    }

    private fun click(button: String, at: Pair<Float, Float>) {
        add(pointer(at))
        listOf(true, false).forEach { add(ev("button", "button" to button, "down" to it)) }
    }

    private fun button(button: String, down: Boolean) {
        if (down) heldButtons.add(button) else heldButtons.remove(button)
        add(ev("button", "button" to button, "down" to down))
    }

    private fun pointer(at: Pair<Float, Float>): JsonObject {
        val (x, y) = toSource(at.first, at.second)
        return ev("pointer_abs", "x" to x, "y" to y)
    }

    private fun ev(type: String, vararg fields: Pair<String, Any>): JsonObject = buildJsonObject {
        put("type", type)
        fields.forEach { (n, v) -> if (v is Boolean) put(n, v) else if (v is Int) put(n, v) else put(n, v.toString()) }
    }

    private fun JsonObject.text(name: String) = (this[name] as? JsonPrimitive)?.content

    private fun add(event: JsonObject) {
        val isMove = event.text("type") == "touch" && event.text("phase") == "move"
        val at = if (isMove) queue.indexOfLast { it.text("type") == "touch" && it["id"] == event["id"] } else -1
        when {
            event.text("type") == "pointer_abs" && queue.lastOrNull()?.text("type") == "pointer_abs" ->
                queue[queue.lastIndex] = event
            isMove && at >= 0 && queue[at].text("phase") == "move" -> queue[at] = event
            else -> queue.add(event)
        }
    }

    fun drain(): List<JsonObject> = ArrayList(queue).also { queue.clear() }

    /** What waits plus the counterpart of all that is pressed (button up, key up, touch cancel). */
    fun releaseAll(): List<JsonObject> {
        heldButtons.reversed().forEach { add(ev("button", "button" to it, "down" to false)) }
        heldButtons.clear()
        heldKeys.reversed().forEach { add(ev("key", "key" to it, "down" to false)) }
        heldKeys.clear()
        cancelTouches()
        mode = Mode.IDLE
        return drain()
    }

    companion object {
        const val MAX_FINGERS = 10
        private const val WHEEL_NOTCH = 120f
        private const val MAX_TEXT_BYTES = 4096
        private val MODIFIERS = setOf("ctrl_left", "ctrl_right", "shift_left", "shift_right", "alt_left", "alt_right")

        /** `KEYCODE_<constant>=name` of the core's `key_usage` for keys without a pattern. */
        private val NAMED: Map<Int, String> by lazy {
            ("DPAD_UP=up DPAD_DOWN=down DPAD_LEFT=left DPAD_RIGHT=right ENTER=enter NUMPAD_ENTER=kp_enter " +
                "DEL=backspace FORWARD_DEL=delete TAB=tab ESCAPE=escape SPACE=space MOVE_HOME=home MOVE_END=end " +
                "PAGE_UP=page_up PAGE_DOWN=page_down INSERT=insert MINUS=minus EQUALS=equal " +
                "LEFT_BRACKET=bracket_left RIGHT_BRACKET=bracket_right BACKSLASH=backslash SEMICOLON=semicolon " +
                "APOSTROPHE=quote GRAVE=grave COMMA=comma PERIOD=period SLASH=slash " +
                "CTRL_LEFT=ctrl_left CTRL_RIGHT=ctrl_right SHIFT_LEFT=shift_left SHIFT_RIGHT=shift_right " +
                "ALT_LEFT=alt_left ALT_RIGHT=alt_right META_LEFT=super_left META_RIGHT=super_right")
                .split(' ')
                .associate { pair ->
                    val (constant, name) = pair.split('=')
                    KeyEvent::class.java.getField("KEYCODE_$constant").getInt(null) to name
                }
        }

        /** The name `key_usage` of the core knows for an Android key code, or `null`. */
        fun keyName(keyCode: Int): String? = when (keyCode) {
            in KeyEvent.KEYCODE_A..KeyEvent.KEYCODE_Z -> ('a' + (keyCode - KeyEvent.KEYCODE_A)).toString()
            in KeyEvent.KEYCODE_0..KeyEvent.KEYCODE_9 -> ('0' + (keyCode - KeyEvent.KEYCODE_0)).toString()
            in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F12 -> "f${keyCode - KeyEvent.KEYCODE_F1 + 1}"
            else -> NAMED[keyCode]
        }
    }
}
