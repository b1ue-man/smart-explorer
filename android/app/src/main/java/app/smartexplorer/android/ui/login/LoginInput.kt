package app.smartexplorer.android.ui.login

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.os.SystemClock
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import app.smartexplorer.android.api.ScreenInputCaps
import app.smartexplorer.android.ui.screen.Finger
import app.smartexplorer.android.ui.screen.InputConfig
import app.smartexplorer.android.ui.screen.InputHost
import app.smartexplorer.android.ui.screen.ScreenInputMapper
import app.smartexplorer.android.ui.screen.TouchKind

/** The bitmap is acknowledged only after onDraw actually draws its fitted bounds. */
class LoginCanvas(context: Context, private val owner: LoginSession) : View(context) {
    private val paint = Paint(Paint.FILTER_BITMAP_FLAG)
    private val rect = RectF()
    private val visible = android.graphics.Rect()
    private var picture: LoginPicture? = null
    private var drawn: app.smartexplorer.android.api.LoginToken? = null
    private var control = false
    private val config = ViewConfiguration.get(context)
    private val mapper = ScreenInputMapper(InputHost.DESKTOP,
        InputConfig(ViewConfiguration.getLongPressTimeout().toLong(), config.scaledTouchSlop.toFloat(), config.scaledVerticalScrollFactor),
        toSource = { x, y ->
            (((x - rect.left) / rect.width()).coerceIn(0f, 1f) * 65535).toInt() to (((y - rect.top) / rect.height()).coerceIn(0f, 1f) * 65535).toInt()
        }, caps = ScreenInputCaps(pointer = true, keyboard = true, text = true))
    private val longPress = object : Runnable {
        override fun run() {
            mapper.onTick(SystemClock.uptimeMillis())
            flush()
        }
    }
    init { isFocusableInTouchMode = true; isFocusable = true; setBackgroundColor(Color.BLACK) }
    fun update(next: LoginPicture?, allowControl: Boolean) {
        if (control && !allowControl) release()
        picture = next; control = allowControl
        owner.focus(hasWindowFocus(), allowControl); invalidate()
    }
    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val image = picture ?: return
        if (width <= 0 || height <= 0 || !isShown || windowVisibility != VISIBLE) return
        val scale = minOf(width.toFloat() / image.bitmap.width, height.toFloat() / image.bitmap.height)
        val w = image.bitmap.width * scale; val h = image.bitmap.height * scale
        rect.set((width - w) / 2, (height - h) / 2, (width + w) / 2, (height + h) / 2)
        canvas.drawBitmap(image.bitmap, null, rect, paint)
        if (getLocalVisibleRect(visible) && visible.left <= rect.left && visible.top <= rect.top && visible.right >= rect.right && visible.bottom >= rect.bottom) {
            drawn = image.token; owner.presented(image.token)
        } else { release() }
    }
    override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
        super.onWindowFocusChanged(hasWindowFocus)
        if (!hasWindowFocus) release()
        owner.focus(hasWindowFocus, control); drawn = null; invalidate()
    }
    override fun onDetachedFromWindow() { release(); super.onDetachedFromWindow() }
    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        if (w != oldw || h != oldh) { release(); owner.focus(hasWindowFocus(), control); invalidate() }
    }
    private fun release() { removeCallbacks(longPress); mapper.releaseAll(); owner.release(); drawn = null }
    private fun flush() {
        val token = drawn; val events = mapper.drain()
        if (token == null || !control || !hasWindowFocus() || !owner.state.value.input_ready) { if (events.isNotEmpty()) release(); return }
        owner.input(token, events)
    }
    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (!control || !hasWindowFocus() || drawn == null || !owner.state.value.input_ready) { release(); return false }
        if ((0 until event.pointerCount).any { !rect.contains(event.getX(it), event.getY(it)) }) { release(); return false }
        requestFocus()
        val kind = when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> TouchKind.DOWN
            MotionEvent.ACTION_POINTER_DOWN -> TouchKind.POINTER_DOWN
            MotionEvent.ACTION_MOVE -> TouchKind.MOVE
            MotionEvent.ACTION_POINTER_UP -> TouchKind.POINTER_UP
            MotionEvent.ACTION_UP -> TouchKind.UP
            MotionEvent.ACTION_CANCEL -> TouchKind.CANCEL
            else -> return false
        }
        val fingers = (0 until event.pointerCount).map { Finger(event.getPointerId(it), event.getX(it), event.getY(it)) }
        mapper.onTouch(kind, event.eventTime, fingers, event.getPointerId(event.actionIndex)); flush()
        if (kind == TouchKind.DOWN) postDelayed(longPress, ViewConfiguration.getLongPressTimeout().toLong())
        if (kind == TouchKind.UP || kind == TouchKind.CANCEL) { removeCallbacks(longPress); performClick() }
        return true
    }
    override fun performClick(): Boolean { super.performClick(); return true }
    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean = key(keyCode, event, true) || super.onKeyDown(keyCode, event)
    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean = key(keyCode, event, false) || super.onKeyUp(keyCode, event)
    private fun key(code: Int, event: KeyEvent, down: Boolean): Boolean {
        if (!control || !hasWindowFocus() || !owner.state.value.input_ready || drawn == null) { release(); return false }
        if (ScreenInputMapper.keyName(code) == null) return false
        if (event.repeatCount == 0) { mapper.onKey(code, down, event.metaState); flush() }
        return true
    }
}
