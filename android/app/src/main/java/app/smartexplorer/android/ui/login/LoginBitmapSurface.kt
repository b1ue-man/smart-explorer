package app.smartexplorer.android.ui.login

import android.content.Context
import android.graphics.RectF
import android.graphics.SurfaceTexture
import android.view.TextureView

/** TextureView contains no source Bitmap in the framework View display list. */
internal class LoginBitmapSurface(context: Context, private val renderer: LoginBitmapRenderer, private val resized: (() -> Unit)? = null, private val changed: () -> Unit) : TextureView(context), TextureView.SurfaceTextureListener {
    init { surfaceTextureListener = this; isOpaque = true; isFocusable = false }
    fun render(picture: LoginPicture, rect: RectF, presented: () -> Unit): Boolean {
        val texture = surfaceTexture ?: return false
        if (!renderer.attach(texture, width, height)) return false
        renderer.render(picture, rect, presented)
        return true
    }
    override fun onSurfaceTextureAvailable(surface: SurfaceTexture, width: Int, height: Int) { changed() }
    override fun onSurfaceTextureSizeChanged(surface: SurfaceTexture, width: Int, height: Int) { if (resized != null) resized.invoke() else changed() }
    override fun onSurfaceTextureDestroyed(surface: SurfaceTexture): Boolean = !renderer.detach(surface)
    override fun onSurfaceTextureUpdated(surface: SurfaceTexture) {
        val observer = viewTreeObserver
        if (!isAttachedToWindow || !observer.isAlive) return
        val committed = renderer.consumerCommit(surface) ?: return
        observer.registerFrameCommitCallback(committed)
        // ViewRoot collects these callbacks before drawing its children; this
        // update happens during draw, so explicitly request the following frame.
        postInvalidateOnAnimation()
    }
}
