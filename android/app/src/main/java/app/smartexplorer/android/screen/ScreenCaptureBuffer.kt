package app.smartexplorer.android.screen

import android.graphics.Bitmap
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Native storage with explicit alias ownership; retain never copies pixels. */
internal class ScreenCaptureBuffer(
    private val storage: ByteBuffer,
    private val lease: ScreenCaptureLease,
    val budget: ScreenCaptureBudget,
) : AutoCloseable {
    private var references=1
    val capacity: Int get()=storage.capacity()
    @Synchronized fun retain(): ScreenCaptureBuffer {check(references>0);references=Math.addExact(references,1);return this}
    /** The view cannot leave this call or be used after close. */
    @Synchronized fun <T> access(use: (ByteBuffer) -> T): T {
        check(references>0)
        return use(storage.duplicate().order(ByteOrder.LITTLE_ENDIAN).apply {clear()})
    }
    fun samePixels(other: ScreenCaptureBuffer): Boolean {
        if(this===other) return true
        // No nested owner locks: use a read-only duplicate while both owners
        // are retained by the encoder. Pixel buffers are immutable after publish.
        val first=access {it.asReadOnlyBuffer()}
        return other.access {second -> first==second}
    }
    @Synchronized override fun close() {
        check(references>0)
        references--
        if(references==0) lease.close()
    }
}

/** The software bitmap dies before its grant; one callback owns the wrapper. */
internal class ScreenCaptureBitmap(val bitmap: Bitmap, private val lease: ScreenCaptureLease) : AutoCloseable {
    override fun close() {try {bitmap.recycle()} finally {lease.close()}}
}
