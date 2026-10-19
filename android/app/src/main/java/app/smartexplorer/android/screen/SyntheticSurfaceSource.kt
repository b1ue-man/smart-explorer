package app.smartexplorer.android.screen

import android.graphics.Color
import android.graphics.Paint
import android.os.SystemClock
import android.view.Surface
import kotlin.concurrent.thread

/**
 * The test picture `synthetic` of this phone as host (C9, SCR-M8): no screen content, so no system
 * dialog and no indicator are needed. Draws [fps] pictures per second into the encoder's input
 * surface: a moving bar and the frame number as 32 black and white columns (bit 31 left), the
 * barcode of the core's test picture, so a viewer can measure order and loss.
 */
internal class SyntheticSurfaceSource(
    private val surface: Surface,
    private val width: Int,
    private val height: Int,
    private val fps: Int,
) {
    @Volatile
    private var running = true

    /** Pictures drawn so far. */
    @Volatile
    var frames: Long = 0
        private set

    private val worker = thread(name = "screen-synthetic", isDaemon = true) { loop() }

    fun stop() {
        running = false
        worker.join(STOP_JOIN_MS)
    }

    private fun loop() {
        val paint = Paint()
        val interval = 1_000L / fps.coerceIn(1, 60)
        while (running) {
            val started = SystemClock.uptimeMillis()
            val canvas = try {
                surface.lockHardwareCanvas()
            } catch (e: RuntimeException) {
                return
            }
            try {
                canvas.drawColor(Color.rgb(32, 64, 128))
                paint.color = Color.rgb(240, 200, 40)
                val x = (frames * 8 % width.coerceAtLeast(1)).toFloat()
                canvas.drawRect(x, 0f, x + width / 16f, height.toFloat(), paint)
                val column = width / 32f
                val barHeight = height / 8f
                for (bit in 0 until 32) {
                    paint.color = if ((frames shr (31 - bit)) and 1L == 1L) Color.WHITE else Color.BLACK
                    canvas.drawRect(bit * column, 0f, (bit + 1) * column, barHeight, paint)
                }
            } finally {
                surface.unlockCanvasAndPost(canvas)
            }
            frames++
            val left = interval - (SystemClock.uptimeMillis() - started)
            if (left > 0) SystemClock.sleep(left)
        }
    }

    private companion object {
        const val STOP_JOIN_MS = 1_000L
    }
}
