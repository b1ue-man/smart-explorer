package app.smartexplorer.android.screen

import android.graphics.Bitmap
import android.os.SystemClock
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Push flag of a still: 8 bytes width and height (little endian), then RGBA rows. */
internal const val FLAG_STILL = 0x2

/** Push flag of a key frame. */
internal const val FLAG_KEYFRAME = 0x1

/** The epoch bits of the push flags (`configure`). */
internal fun epochFlags(epoch: Int): Int = (epoch and 0xFFFF) shl 16

/**
 * Still pictures of this phone as host (C1 Android L2): the accessibility service's screenshot of
 * the whole display, never more than one per [MIN_INTERVAL_MS] (Android refuses faster ones),
 * packed for `screenPush` with [FLAG_STILL]. Secure windows show black (A12). Used for tile
 * previews and snapshots without the system dialog; nothing of a still is kept or logged.
 */
internal class ScreenStillSource(private val sink: (ByteBuffer, Int) -> Int) {
    @Volatile private var active=true
    @Volatile private var generation=0L
    @Synchronized fun setCaptureActive(value: Boolean) {generation=Math.addExact(generation,1);active=value}
    private var lastAt = 0L
    private var buffer: ByteBuffer = ByteBuffer.allocateDirect(0)

    /** Milliseconds until the next still may be taken: [MIN_INTERVAL_MS] and [MARGIN_MS] after the last request. */
    fun waitMs(now: Long = SystemClock.uptimeMillis()): Long =
        if (lastAt == 0L) 0 else (MIN_INTERVAL_MS + MARGIN_MS - (now - lastAt)).coerceAtLeast(0)

    /**
     * Takes one still through [service] and pushes it; [done] gets the push code, or `null` when
     * Android gave no picture. Waits for the interval first.
     */
    fun capture(service: ScreenControlService, done: (Int?) -> Unit) {
        val admittedGeneration=generation
        if(!active) {done(null);return}
        val wait = waitMs()
        if (wait > 0) SystemClock.sleep(wait)
        synchronized(this) {
            if(!active || generation!=admittedGeneration) {done(null);return}
            lastAt = SystemClock.uptimeMillis()
            service.still { bitmap ->
                if (bitmap == null) {
                    done(null)
                } else {
                    try {done(synchronized(this) {if(active && generation==admittedGeneration) push(bitmap) else null})}
                    finally {bitmap.recycle()}
                }
            }
        }
    }

    /** Packs [bitmap] (ARGB_8888: R, G, B, A in memory) and pushes it. */
    fun push(bitmap: Bitmap): Int {
        val expected=try {ScreenFrameLimits.checkedBytes(bitmap.width,bitmap.height).toLong()} catch (_: IllegalArgumentException) {return -2}
        val compatible=if(bitmap.config==Bitmap.Config.ARGB_8888) bitmap else bitmap.copy(Bitmap.Config.ARGB_8888,false) ?: return -2
        try {
            if(compatible.byteCount.toLong()!=expected) return -2
            val total=Math.addExact(HEADER,expected.toInt())
            if(buffer.capacity()<total) buffer=ByteBuffer.allocateDirect(total)
            buffer.clear(); buffer.order(ByteOrder.LITTLE_ENDIAN)
            buffer.putInt(compatible.width); buffer.putInt(compatible.height)
            compatible.copyPixelsToBuffer(buffer)
            return sink(buffer,total)
        } finally { if(compatible!==bitmap) compatible.recycle() }
    }

    companion object {
        /** The interval Android demands between two stills (`ACCESSIBILITY_TAKE_SCREENSHOT_REQUEST_INTERVAL_TIMES_MS`). */
        const val MIN_INTERVAL_MS = 333L

        /**
         * Android refuses a request when at most [MIN_INTERVAL_MS] passed since the previous one
         * (`AbstractAccessibilityServiceConnection.takeScreenshot`, `<=`), counted from the time each
         * request reaches the system process. The binder call of the second request can be faster
         * than that of the first, so the next request waits this much longer than the interval.
         */
        const val MARGIN_MS = 50L
        const val HEADER = 8
    }
}
