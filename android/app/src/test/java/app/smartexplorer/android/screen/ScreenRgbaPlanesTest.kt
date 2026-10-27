package app.smartexplorer.android.screen

import java.nio.ByteBuffer
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertThrows
import org.junit.Test

/** The row-wise host transfers against the former per-pixel loops (bit-identical results). */
class ScreenRgbaPlanesTest {
    private fun rgba(width: Int, height: Int, seed: Int): ByteArray =
        ByteArray(width * height * 4) { ((it * 37 + seed * 101 + it / 7) and 255).toByte() }

    /** Lays [pixels] out as an ImageReader plane: [rowStride], [pixelStride], data ends after the last pixel. */
    private fun plane(pixels: ByteArray, width: Int, height: Int, rowStride: Int, pixelStride: Int, offset: Int = 0): ScreenPlane {
        val size = offset + (height - 1) * rowStride + (width - 1) * pixelStride + 4
        val buffer = ByteBuffer.allocate(size)
        for (y in 0 until height) for (x in 0 until width) for (c in 0..3)
            buffer.put(offset + y * rowStride + x * pixelStride + c, pixels[(y * width + x) * 4 + c])
        buffer.position(offset)
        return ScreenPlane(buffer, rowStride, pixelStride)
    }

    private fun copied(plane: ScreenPlane, width: Int, height: Int): ByteArray {
        val target = ByteBuffer.allocate(width * height * 4)
        ScreenRgbaPlanes.copyRgba(plane, width, height, target)
        return target.array()
    }

    @Test fun screen_metadata_runtime_task_rgbaCopyKeepsEveryPixelForTightPaddedAndOffsetPlanes() {
        val width = 7
        val height = 5
        val pixels = rgba(width, height, 3)
        assertArrayEquals(pixels, copied(plane(pixels, width, height, width * 4, 4), width, height))
        // Row padding as ImageReader delivers it, data ending after the last pixel of the last row.
        assertArrayEquals(pixels, copied(plane(pixels, width, height, width * 4 + 36, 4), width, height))
        assertArrayEquals(pixels, copied(plane(pixels, width, height, width * 4 + 12, 4, offset = 9), width, height))
        // A wider pixel stride still copies the four RGBA bytes of each pixel.
        assertArrayEquals(pixels, copied(plane(pixels, width, height, width * 8, 8), width, height))
    }

    @Test fun screen_metadata_runtime_task_rgbaCopyRefusesPlanesShorterThanTheirRows() {
        val width = 4
        val height = 3
        val short = ScreenPlane(ByteBuffer.allocate(width * 4 * height - 1), width * 4, 4)
        assertThrows(IllegalArgumentException::class.java) { copied(short, width, height) }
        val overlapping = ScreenPlane(ByteBuffer.allocate(1024), width * 4 - 4, 4)
        assertThrows(IllegalArgumentException::class.java) { copied(overlapping, width, height) }
    }

    /** The former per-pixel conversion of ScreenEncoder.queueRgba, verbatim in its arithmetic. */
    private fun reference(pixels: ByteArray, width: Int, height: Int, planes: Array<ScreenPlane>, left: Int, top: Int) {
        fun sample(plane: Int, x: Int, y: Int, value: Int) {
            val p = planes[plane]
            p.buffer.put(p.buffer.position() + y * p.rowStride + x * p.pixelStride, value.coerceIn(0, 255).toByte())
        }
        for (y in 0 until height) for (x in 0 until width) {
            val at = (y * width + x) * 4
            val r = pixels[at].toInt() and 255; val g = pixels[at + 1].toInt() and 255; val b = pixels[at + 2].toInt() and 255
            sample(0, x + left, y + top, ((47 * r + 157 * g + 16 * b + 128) shr 8) + 16)
        }
        for (y in 0 until height step 2) for (x in 0 until width step 2) {
            var r = 0; var g = 0; var b = 0
            for (dy in 0..1) for (dx in 0..1) {
                val at = (minOf(y + dy, height - 1) * width + minOf(x + dx, width - 1)) * 4
                r += pixels[at].toInt() and 255; g += pixels[at + 1].toInt() and 255; b += pixels[at + 2].toInt() and 255
            }
            sample(1, (x + left) / 2, (y + top) / 2, ((-26 * r - 86 * g + 112 * b + 512) shr 10) + 128)
            sample(2, (x + left) / 2, (y + top) / 2, ((112 * r - 102 * g - 10 * b + 512) shr 10) + 128)
        }
    }

    /** Planar (I420, chroma pixel stride 1) or semi-planar (NV12: U and V interleaved in one buffer). */
    private fun yuvPlanes(width: Int, height: Int, semiPlanar: Boolean, yStride: Int): Pair<Array<ScreenPlane>, List<ByteBuffer>> {
        val y = ByteBuffer.allocate(yStride * height)
        val chromaRows = (height + 1) / 2
        return if (semiPlanar) {
            val stride = yStride
            val uv = ByteBuffer.allocate(stride * chromaRows)
            val v = uv.duplicate().apply { position(1) }
            arrayOf(ScreenPlane(y, yStride, 1), ScreenPlane(uv, stride, 2), ScreenPlane(v, stride, 2)) to listOf(y, uv)
        } else {
            val stride = (yStride + 1) / 2
            val u = ByteBuffer.allocate(stride * chromaRows)
            val v = ByteBuffer.allocate(stride * chromaRows)
            arrayOf(ScreenPlane(y, yStride, 1), ScreenPlane(u, stride, 1), ScreenPlane(v, stride, 1)) to listOf(y, u, v)
        }
    }

    @Test fun screen_metadata_runtime_task_yuvConversionMatchesThePerPixelLoopBitForBit() {
        for ((width, height, left, top) in listOf(listOf(8, 6, 0, 0), listOf(7, 5, 0, 0), listOf(9, 7, 2, 2), listOf(6, 3, 1, 1))) {
            for (semiPlanar in listOf(false, true)) {
                val pixels = rgba(width, height, width * 13 + height)
                val yStride = left + width + 10
                val (expectedPlanes, expectedBuffers) = yuvPlanes(width, height + top, semiPlanar, yStride)
                val (actualPlanes, actualBuffers) = yuvPlanes(width, height + top, semiPlanar, yStride)
                reference(pixels, width, height, expectedPlanes, left, top)
                ScreenRgbaPlanes.rgbaToYuv420(ByteBuffer.wrap(pixels), width, height, actualPlanes[0], actualPlanes[1], actualPlanes[2], left, top)
                for (i in expectedBuffers.indices) assertArrayEquals("${width}x$height+$left+$top nv12=$semiPlanar plane $i",
                    expectedBuffers[i].array(), actualBuffers[i].array())
            }
        }
    }

    @Test fun screen_metadata_runtime_task_yuvConversionRefusesPlanesTooSmallForThePicture() {
        val (planes, _) = yuvPlanes(8, 6, false, 8)
        assertThrows(IllegalArgumentException::class.java) {
            ScreenRgbaPlanes.rgbaToYuv420(ByteBuffer.wrap(rgba(8, 6, 1)), 8, 6, planes[0], planes[1], planes[2], 2, 0)
        }
    }
}
