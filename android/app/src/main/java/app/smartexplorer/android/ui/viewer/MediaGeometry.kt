package app.smartexplorer.android.ui.viewer

import androidx.compose.ui.geometry.Offset
import kotlin.math.max
import kotlin.math.min

/** Geometry uses the fitted image, not its letterboxed page. Focus is in image coordinates. */
internal data class MediaGeometry(val width: Float, val height: Float, val pageWidth: Float, val pageHeight: Float) {
    fun offset(zoom: Float, focus: Offset): Offset = clamp(
        Offset((0.5f - focus.x) * width * zoom, (0.5f - focus.y) * height * zoom), zoom,
    )
    fun focus(zoom: Float, offset: Offset): Offset = Offset(
        (0.5f - offset.x / (width * zoom)).coerceIn(0f, 1f),
        (0.5f - offset.y / (height * zoom)).coerceIn(0f, 1f),
    )
    fun clamp(value: Offset, zoom: Float): Offset {
        val x = max(0f, (width * zoom - pageWidth) / 2f)
        val y = max(0f, (height * zoom - pageHeight) / 2f)
        return Offset(value.x.coerceIn(-x, x), value.y.coerceIn(-y, y))
    }
    companion object {
        fun fit(imageWidth: Int, imageHeight: Int, pageWidth: Float, pageHeight: Float): MediaGeometry {
            val factor = min(pageWidth / imageWidth.coerceAtLeast(1), pageHeight / imageHeight.coerceAtLeast(1))
            return MediaGeometry(imageWidth * factor, imageHeight * factor, pageWidth, pageHeight)
        }
    }
}
