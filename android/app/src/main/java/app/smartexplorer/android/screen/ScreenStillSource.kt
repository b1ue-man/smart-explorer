package app.smartexplorer.android.screen

import android.os.SystemClock
import java.nio.ByteBuffer

/** Push flag of a still: 8 bytes width and height (little endian), then RGBA rows. */
internal const val FLAG_STILL = 0x2
internal const val FLAG_KEYFRAME = 0x1
internal fun epochFlags(epoch: Int): Int = (epoch and 0xFFFF) shl 16

/** One admitted accessibility request; callbacks retain their own generation after close. */
internal class ScreenStillSource(
    private val session: Long,
    private val geometry: () -> Pair<Int,Int>,
    private val sink: (ByteBuffer, Int, Long) -> Int,
) : AutoCloseable {
    @Volatile private var active=true
    @Volatile private var generation=0L
    private var inFlight=false
    private var size: Pair<Int,Int>?=null
    private var budget: ScreenCaptureBudget?=null
    private var buffer: ScreenCaptureBuffer?=null
    private var lastAt=0L
    @Synchronized fun setCaptureActive(value: Boolean) {
        generation=Math.addExact(generation,1);active=value
        if(!value) {buffer?.close();buffer=null;budget=null;size=null}
    }
    override fun close() = setCaptureActive(false)
    fun waitMs(now: Long=SystemClock.uptimeMillis()): Long =
        if(lastAt==0L) 0 else (MIN_INTERVAL_MS+MARGIN_MS-(now-lastAt)).coerceAtLeast(0)

    fun capture(service: ScreenControlService, done: (Int?) -> Unit) {
        val admittedGeneration=generation
        if(!active) {done(null);return}
        val wait=waitMs()
        if(wait>0) SystemClock.sleep(wait)
        synchronized(this) {
            if(!active || generation!=admittedGeneration || inFlight) {done(null);return}
            val expected=geometry()
            val current=if(size==expected) budget else null
            val admitted=current ?: try {
                ScreenCaptureBudget.open(session,expected.first,expected.second,ScreenCaptureBudget.ACCESSIBILITY).also {
                    buffer?.close();buffer=null;budget=it;size=expected
                }
            } catch(_: IllegalStateException) {done(null);return}
            lastAt=SystemClock.uptimeMillis();inFlight=true
            try {
                // Both requested hardware and software claims precede takeScreenshot.
                service.still(admitted,expected.first,expected.second) {result ->
                    try {
                        val code=if(result==null) null else synchronized(this) {
                            if(active && generation==admittedGeneration) push(result,admitted) else null
                        }
                        done(code)
                    } finally {result?.close();synchronized(this) {inFlight=false}}
                }
            } catch(error: RuntimeException) {inFlight=false;done(null)}
        }
    }
    private fun push(result: ScreenCaptureBitmap, admitted: ScreenCaptureBudget): Int {
        val bitmap=result.bitmap
        val expected=ScreenFrameLimits.checkedBytes(bitmap.width,bitmap.height)
        if(bitmap.config!=android.graphics.Bitmap.Config.ARGB_8888 || bitmap.byteCount!=expected) return -2
        val total=Math.addExact(HEADER,expected)
        val packet=buffer?.takeIf {it.capacity==total && it.budget===admitted} ?: admitted.buffer(ScreenCaptureBudget.TRANSPORT,total).also {
            buffer?.close();buffer=it
        }
        return packet.access {target ->
            target.putInt(bitmap.width);target.putInt(bitmap.height);bitmap.copyPixelsToBuffer(target)
            sink(target,total,admitted.generation)
        }
    }
    companion object {
        const val MIN_INTERVAL_MS=333L
        const val MARGIN_MS=50L
        const val HEADER=8
    }
}
