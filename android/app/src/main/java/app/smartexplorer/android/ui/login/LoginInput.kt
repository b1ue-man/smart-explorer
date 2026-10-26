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
import android.widget.FrameLayout
import android.view.ViewConfiguration
import app.smartexplorer.android.api.ScreenInputCaps
import app.smartexplorer.android.ui.screen.Finger
import app.smartexplorer.android.ui.screen.InputConfig
import app.smartexplorer.android.ui.screen.InputHost
import app.smartexplorer.android.ui.screen.ScreenInputMapper
import app.smartexplorer.android.ui.screen.TouchKind

/** Input/IME coordinates stay in the containing View; pixels have an explicit renderer owner. */
class LoginCanvas(context: Context, private val owner: LoginSession) : FrameLayout(context) {
    private val paint = Paint(Paint.FILTER_BITMAP_FLAG)
    private val rect = RectF()
    private val visible = android.graphics.Rect()
    private var picture: LoginPicture? = null
    private var drawn: app.smartexplorer.android.api.LoginToken? = null
    private var control = false
    private var generation = 1L
    private var requested: Pair<LoginPictureDescriptor, LoginPresentationStamp>? = null
    private val renderer = LoginBitmapRenderer(owner.images) {
        post { if (isAttachedToWindow && owner.state.value.state !in listOf("ended", "stopping")) { resetHardwareSurface(); submitHardware() } }
    }
    private var surface = newSurface()
    private fun newSurface() = LoginBitmapSurface(context, renderer, resized = {
        post { if (isAttachedToWindow) { resetHardwareSurface(); submitHardware() } }
    }) { requested = null; submitHardware() }
    private fun resetHardwareSurface() {
        renderer.close()
        removeView(surface)
        surface = newSurface()
        addView(surface, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        requested = null
    }
    private val retryRender = Runnable { requested = null; submitHardware() }
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
    init {
        isFocusableInTouchMode = true; isFocusable = true
        setBackgroundColor(Color.BLACK); setWillNotDraw(false)
        clipChildren = true; clipToPadding = true
        owner.registerRenderer(renderer)
        addView(surface, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
    }
    fun update(next: LoginPicture?, allowControl: Boolean) {
        if (control && !allowControl) release()
        if (picture?.identity != next?.identity) {
            val old = picture
            picture = next?.retain()
            old?.close()
            requested = null
        }
        surface.visibility = if (picture == null) INVISIBLE else VISIBLE
        control = allowControl
        owner.focus(hasWindowFocus(), allowControl)
        if (picture == null) { generation++; requested = null; drawn = null; renderer.clear() }
        submitHardware(); invalidate()
    }
    private fun fit(image: LoginPicture) {
        val scale = minOf(width.toFloat() / image.token.width, height.toFloat() / image.token.height)
        val w = image.token.width * scale; val h = image.token.height * scale
        rect.set((width - w) / 2, (height - h) / 2, (width + w) / 2, (height + h) / 2)
    }
    private fun fullyVisible(): Boolean = width > 0 && height > 0 && isShown && isAttachedToWindow && windowVisibility == VISIBLE &&
        getLocalVisibleRect(visible) && visible.left <= rect.left && visible.top <= rect.top && visible.right >= rect.right && visible.bottom >= rect.bottom
    private fun confirm(image: LoginPicture, stamp: LoginPresentationStamp, expectedGeneration: Long) {
        if (generation != expectedGeneration || picture?.identity != image.identity || stamp != owner.presentationStamp()) return
        if (!fullyVisible() || !hasWindowFocus()) { if (drawn != null) release(); return }
        drawn = image.token; owner.presented(image.token, stamp)
    }
    private fun submitHardware() {
        if (owner.state.value.state in listOf("ended", "stopping") || !isHardwareAccelerated) return
        val image = picture ?: return
        if (width <= 0 || height <= 0) return
        fit(image)
        if (!fullyVisible()) { if (drawn != null) release(); return }
        val stamp = owner.presentationStamp()
        val request = image.identity to stamp
        if (requested == request) return
        requested = request
        val expectedGeneration = generation
        if (!surface.render(image, rect) { post { confirm(image, stamp, expectedGeneration) } }) {
            requested = null
            removeCallbacks(retryRender); postDelayed(retryRender, 25)
        }
    }
    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        if (owner.state.value.state in listOf("ended", "stopping")) return
        if (canvas.isHardwareAccelerated) { submitHardware(); return }
        // A real software Canvas consumes pixels synchronously. No software layer
        // is forced and no Bitmap is recorded into a framework hardware display list.
        val image = picture ?: return
        if (width <= 0 || height <= 0) return
        fit(image)
        if (!fullyVisible()) { if (drawn != null) release(); return }
        val stamp = owner.presentationStamp(); val expectedGeneration = generation
        val borrow = image.borrow() ?: return
        try { borrow.draw(canvas, rect, paint) } finally { borrow.close() }
        confirm(image, stamp, expectedGeneration)
    }
    override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
        super.onWindowFocusChanged(hasWindowFocus)
        if (!hasWindowFocus) { release(); resetHardwareSurface() }
        owner.focus(hasWindowFocus, control); drawn = null; requested = null; submitHardware(); invalidate()
    }
    override fun onDetachedFromWindow() {
        release(); renderer.close(); owner.unregisterRenderer(renderer); picture?.close(); picture = null
        super.onDetachedFromWindow()
    }
    override fun onAttachedToWindow() { super.onAttachedToWindow(); owner.registerRenderer(renderer); if (picture == null) picture = owner.picture.value?.retain(); requested = null; invalidate() }
    override fun onVisibilityChanged(changedView: View, visibility: Int) {
        super.onVisibilityChanged(changedView, visibility)
        if (!isAttachedToWindow) return
        if (visibility != VISIBLE) { release(); resetHardwareSurface() } else { owner.focus(hasWindowFocus(), control); requested = null; invalidate() }
    }
    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        if (w != oldw || h != oldh) { release(); resetHardwareSurface(); owner.focus(hasWindowFocus(), control); requested = null; submitHardware(); invalidate() }
    }
    private fun release() { removeCallbacks(retryRender); generation++; requested = null; removeCallbacks(longPress); mapper.releaseAll(); owner.release(); drawn = null }
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
