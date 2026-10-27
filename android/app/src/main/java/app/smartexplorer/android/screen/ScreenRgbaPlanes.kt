package app.smartexplorer.android.screen

import java.nio.ByteBuffer

/**
 * One plane as `android.media.Image.Plane` describes it: the plane's buffer (its data starts at the
 * buffer's position), the distance of two rows and of two samples in a row, in bytes.
 */
internal class ScreenPlane(val buffer: ByteBuffer, val rowStride: Int, val pixelStride: Int)

/**
 * Row-wise pixel transfers of the phone host (MediaProjection ImageReader → RGBA rows, RGBA rows →
 * the encoder's YUV 4:2:0 input image). A per-pixel loop over a 1080×2400 display costs millions of
 * ByteBuffer index calls per frame; and `MediaCodec`'s `Image.getPlanes()` returns a fresh array
 * copy on every call (AOSP `MediaCodec.MediaImage.getPlanes`: `Arrays.copyOf(mPlanes, ...)`), so it
 * must be read once per image, never per sample.
 */
internal object ScreenRgbaPlanes {
    /**
     * Copies [width]×[height] RGBA pixels of [plane] (`PixelFormat.RGBA_8888`, one plane) tightly
     * packed into [target] from its position. Rows are bulk copies; the bytes after the last pixel
     * of the last row need not exist (`Image.Plane.getBuffer`: data is only guaranteed up to the last
     * pixel in the last row). The alpha byte is copied as delivered: every consumer of these rows
     * ignores it (the native still and hybrid decoders set it to 255, the YUV conversion skips it).
     */
    fun copyRgba(plane: ScreenPlane, width: Int, height: Int, target: ByteBuffer) {
        require(width > 0 && height > 0 && plane.pixelStride >= 4 && plane.rowStride > 0)
        require(plane.rowStride.toLong() >= (width - 1).toLong() * plane.pixelStride + 4)
        val row = Math.multiplyExact(width, 4)
        require(target.remaining().toLong() >= row.toLong() * height)
        val input = plane.buffer
        val base = input.position()
        val end = base.toLong() + (height - 1).toLong() * plane.rowStride + (width - 1).toLong() * plane.pixelStride + 4
        require(end <= input.limit()) { "RGBA-Ebene kürzer als ihre Zeilen" }
        val source = input.duplicate()
        fun take(from: Int, length: Int) {
            source.clear()
            source.position(from)
            source.limit(from + length)
            target.put(source)
        }
        when {
            plane.pixelStride == 4 && plane.rowStride == row -> take(base, row * height)
            plane.pixelStride == 4 -> for (y in 0 until height) take(base + y * plane.rowStride, row)
            else -> for (y in 0 until height) for (x in 0 until width) take(base + y * plane.rowStride + x * plane.pixelStride, 4)
        }
    }

    /**
     * Converts tightly packed RGBA rows ([rgba] from index 0, alpha ignored) into BT.709
     * limited-range YUV 4:2:0 ([y], [u], [v]: the encoder's `COLOR_FormatYUV420Flexible` input image,
     * planar or semi-planar), placing the picture at ([left], [top]) of the image (its crop
     * rectangle). Chroma is the mean of each 2×2 block (edge pixels repeated). The integer formulas
     * are the ones of the former per-pixel loop, so the output is bit-identical.
     */
    fun rgbaToYuv420(rgba: ByteBuffer, width: Int, height: Int, y: ScreenPlane, u: ScreenPlane, v: ScreenPlane, left: Int, top: Int) {
        require(width > 0 && height > 0 && left >= 0 && top >= 0)
        val row = Math.multiplyExact(width, 4)
        require(rgba.limit().toLong() >= row.toLong() * height)
        fun last(plane: ScreenPlane, x: Int, yy: Int): Long {
            require(plane.rowStride > 0 && plane.pixelStride > 0)
            return plane.buffer.position().toLong() + yy.toLong() * plane.rowStride + x.toLong() * plane.pixelStride
        }
        require(last(y, left + width - 1, top + height - 1) < y.buffer.limit()) { "Y-Ebene zu klein" }
        require(last(u, (left + width - 1) / 2, (top + height - 1) / 2) < u.buffer.limit()) { "U-Ebene zu klein" }
        require(last(v, (left + width - 1) / 2, (top + height - 1) / 2) < v.buffer.limit()) { "V-Ebene zu klein" }
        val source = rgba.duplicate()
        val first = ByteArray(row)
        val second = ByteArray(row)
        val luma = ByteArray(width)
        val yOut = y.buffer.duplicate()
        val yBase = y.buffer.position()
        fun read(line: Int, into: ByteArray) {
            source.clear()
            source.position(line * row)
            source.get(into, 0, row)
        }
        fun writeLuma(line: Int, from: ByteArray) {
            for (x in 0 until width) {
                val at = x * 4
                val r = from[at].toInt() and 255
                val g = from[at + 1].toInt() and 255
                val b = from[at + 2].toInt() and 255
                luma[x] = (((47 * r + 157 * g + 16 * b + 128) shr 8) + 16).coerceIn(0, 255).toByte()
            }
            val start = yBase + (line + top) * y.rowStride + left * y.pixelStride
            if (y.pixelStride == 1) {
                yOut.clear()
                yOut.position(start)
                yOut.put(luma, 0, width)
            } else {
                for (x in 0 until width) yOut.put(start + x * y.pixelStride, luma[x])
            }
        }
        var line = 0
        while (line < height) {
            read(line, first)
            writeLuma(line, first)
            val lower = if (line + 1 < height) {
                read(line + 1, second)
                writeLuma(line + 1, second)
                second
            } else first
            val cy = (line + top) / 2
            var x = 0
            while (x < width) {
                val a = x * 4
                val c = minOf(x + 1, width - 1) * 4
                val r = (first[a].toInt() and 255) + (first[c].toInt() and 255) +
                    (lower[a].toInt() and 255) + (lower[c].toInt() and 255)
                val g = (first[a + 1].toInt() and 255) + (first[c + 1].toInt() and 255) +
                    (lower[a + 1].toInt() and 255) + (lower[c + 1].toInt() and 255)
                val b = (first[a + 2].toInt() and 255) + (first[c + 2].toInt() and 255) +
                    (lower[a + 2].toInt() and 255) + (lower[c + 2].toInt() and 255)
                val cx = (x + left) / 2
                u.buffer.put(u.buffer.position() + cy * u.rowStride + cx * u.pixelStride,
                    (((-26 * r - 86 * g + 112 * b + 512) shr 10) + 128).coerceIn(0, 255).toByte())
                v.buffer.put(v.buffer.position() + cy * v.rowStride + cx * v.pixelStride,
                    (((112 * r - 102 * g - 10 * b + 512) shr 10) + 128).coerceIn(0, 255).toByte())
                x += 2
            }
            line += 2
        }
    }
}
