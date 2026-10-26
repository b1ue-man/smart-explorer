package app.smartexplorer.android.screen

import app.smartexplorer.android.core.NativeBridge
import java.util.concurrent.atomic.AtomicBoolean

/** A generation of the existing Rust session authority, never a second heap pool. */
internal class ScreenCaptureBudget private constructor(val session: Long, val generation: Long) {
    fun claim(owner: Int, bytes: Long): ScreenCaptureLease {
        val token=NativeBridge.screenCaptureClaim(session,generation,owner,bytes,false)
        check(token>0) { "Bildspeicher nicht zugelassen ($bytes Byte, Besitzer $owner)" }
        return adoptCaptureOwner(token,{NativeBridge.screenCaptureRelease(it)}) {ScreenCaptureLease(token)}
    }
    fun tryBuffer(owner: Int, bytes: Int): ScreenCaptureBuffer? {
        require(bytes>0)
        val token=NativeBridge.screenCaptureClaim(session,generation,owner,bytes.toLong(),true)
        if(token<=0) return null
        try {
            val buffer=NativeBridge.screenCaptureBuffer(token) ?: error("Zugelassener Bildpuffer fehlt")
            return ScreenCaptureBuffer(buffer,ScreenCaptureLease(token),this)
        } catch(error: Throwable) {NativeBridge.screenCaptureRelease(token);throw error}
    }
    fun buffer(owner: Int, bytes: Int): ScreenCaptureBuffer =
        checkNotNull(tryBuffer(owner,bytes)) { "Bildpuffer nicht zugelassen ($bytes Byte, Besitzer $owner)" }
    companion object {
        const val NATIVE_SURFACE=0
        const val READBACK=1
        const val SOURCE_PIXELS=2
        const val METADATA=6
        const val TRANSPORT=7
        const val CODEC_INPUT=8
        const val CODEC_OUTPUT=9
        const val ACCESSIBILITY=0
        const val PROJECTION_TILES=1
        const val PROJECTION_HYBRID=2
        const val PROJECTION_SURFACE=3
        const val SYNTHETIC_SURFACE=4
        fun open(session: Long, width: Int, height: Int, kind: Int): ScreenCaptureBudget {
            ScreenFrameLimits.checkedBytes(width,height)
            val generation=NativeBridge.screenCaptureBegin(session,width,height,kind)
            check(generation>0) { "Capture wartet auf seine Speicherzulassung" }
            return ScreenCaptureBudget(session,generation)
        }
    }
}

/** Released after the external resource or native allocation has actually ended. */
internal class ScreenCaptureLease private constructor(private val state: ScreenCaptureReferences) : AutoCloseable {
    private val closed=AtomicBoolean(false)
    constructor(token: Long,release:(Long)->Unit={NativeBridge.screenCaptureRelease(it)}):this(ScreenCaptureReferences(token,release))
    fun retain(): ScreenCaptureLease = synchronized(state) {
        check(!closed.get());state.retain {ScreenCaptureLease(state)}
    }
    override fun close() {
        if(!closed.compareAndSet(false,true)) return
        state.close()
    }
}
