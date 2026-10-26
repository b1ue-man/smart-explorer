package app.smartexplorer.android.ui.login

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import app.smartexplorer.android.api.LoginToken
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.NativeBridge
import app.smartexplorer.android.screen.ScreenFrameLimits
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.serialization.Serializable

@Serializable internal data class LoginPictureDescriptor(val owner: Long, val picture: Long, val generation: Long, val token: LoginToken)

/** One pane survives reconnects. Its worker also owns cleanup after Compose disposal. */
private val loginRenderWorker = Executors.newSingleThreadExecutor { work -> Thread(work, "login-bitmap-owner").apply { isDaemon = true } }

internal interface LoginBitmapCalls {
    fun borrow(image: LoginPictureDescriptor): Bitmap?
    fun endBorrow(image: LoginPictureDescriptor)
    fun retire(image: LoginPictureDescriptor)
    fun claimTarget(owner: Long, width: Int, height: Int): Long
    fun releaseTarget(owner: Long, target: Long)
}
internal object NativeLoginBitmapCalls : LoginBitmapCalls {
    override fun borrow(image: LoginPictureDescriptor) = NativeBridge.loginBitmapBorrow(image.owner, image.picture, image.generation)
    override fun endBorrow(image: LoginPictureDescriptor) = NativeBridge.loginBitmapEndBorrow(image.owner, image.picture, image.generation)
    override fun retire(image: LoginPictureDescriptor) = NativeBridge.loginBitmapRetire(image.owner, image.picture, image.generation)
    override fun claimTarget(owner: Long, width: Int, height: Int) = NativeBridge.loginRenderTargetClaim(owner, width, height)
    override fun releaseTarget(owner: Long, target: Long) = NativeBridge.loginRenderTargetRelease(owner, target)
}

class LoginImagePane internal constructor(internal val calls: LoginBitmapCalls = NativeLoginBitmapCalls, initialHandle: Long = 0) {
    internal val worker = loginRenderWorker
    private var closed = false
    @Volatile internal var handle = initialHandle
        private set
    suspend fun openHandle(): Long {
        Core.call("login.status") // waits for the library/runtime before the first binary JNI call
        return synchronized(this) {
            check(!closed) { "Loginbildfläche wurde geschlossen." }
            if (handle == 0L) handle = NativeBridge.loginImageOwnerCreate(ScreenFrameLimits.workingBytes())
            check(handle > 0) { "Speicherzusage für Loginbilder fehlt." }
            handle
        }
    }
    fun close() {
        synchronized(this) {
            if (closed) return
            closed = true
            if (handle > 0) NativeBridge.loginImageOwnerClose(handle)
        }
        // Views submit retirement before their final worker action. No UI join.
        // Keep this bounded pane executor usable by late TextureView detach callbacks.
    }
    internal fun take(session: Long): LoginPicture? {
        val raw = NativeBridge.loginBitmapTake(session, handle) ?: error("Native Loginbildübernahme fehlgeschlagen.")
        if (raw.isEmpty()) return null
        val descriptor = Core.json.decodeFromString<LoginPictureDescriptor>(raw)
        require(descriptor.owner == handle && descriptor.picture > 0 && descriptor.generation > 0)
        return LoginPicture(LoginBitmapOwner(descriptor))
    }
}

/** Opaque immutable image; consumers cannot obtain a writable Bitmap. */
class LoginPicture internal constructor(private val owner: LoginBitmapOwner) {
    val token: LoginToken get() = owner.descriptor.token
    internal val identity: LoginPictureDescriptor get() = owner.descriptor
    internal fun retain(): LoginPicture? = if (owner.retain()) LoginPicture(owner) else null
    internal fun borrow(): LoginDrawBorrow? = owner.borrow()
    private val closed = AtomicBoolean(false)
    internal fun close() { if (closed.compareAndSet(false, true)) owner.release() }
}

internal class LoginBitmapOwner(val descriptor: LoginPictureDescriptor, private val calls: LoginBitmapCalls = NativeLoginBitmapCalls) {
    private var references = 1
    @Synchronized fun retain(): Boolean {
        if (references == 0) return false
        references = Math.addExact(references, 1)
        return true
    }
    @Synchronized fun release() {
        check(references > 0)
        references--
        if (references == 0) loginRenderWorker.execute { calls.retire(descriptor) }
    }
    @Synchronized fun borrow(): LoginDrawBorrow? {
        if (references == 0) return null
        val bitmap = calls.borrow(descriptor) ?: return null
        return LoginDrawBorrow(bitmap, descriptor, calls)
    }
}

/** Hardware holds this borrow through clearContent/destroy; software closes after drawBitmap. */
internal class LoginDrawBorrow(private val bitmap: Bitmap, private val descriptor: LoginPictureDescriptor, private val calls: LoginBitmapCalls) : AutoCloseable {
    private val closed = AtomicBoolean(false)
    fun draw(canvas: Canvas, rect: RectF, paint: Paint) {
        check(!closed.get())
        canvas.drawBitmap(bitmap, null, rect, paint)
    }
    override fun close() {
        if (closed.compareAndSet(false, true)) calls.endBorrow(descriptor)
    }
}
