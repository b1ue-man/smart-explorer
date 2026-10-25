package app.smartexplorer.android.ui.viewer

import android.graphics.ImageDecoder
import androidx.compose.foundation.Image
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.rememberTransformableState
import androidx.compose.foundation.gestures.transformable
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import java.io.File
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

private const val MAX_ZOOM = 5f
private const val DOUBLE_TAP_ZOOM = 2.5f

/** Decode state of an image page. */
private sealed interface Decoded {
    data object Pending : Decoded

    data class Bitmap(val image: ImageBitmap) : Decoded

    data object Unsupported : Decoded
}

/**
 * One picture, fitted to the page. Two fingers or a double tap zoom; while zoomed one finger
 * moves the picture and [onZoomed] reports `true` so the pager stops paging. A single tap calls
 * [onTap]. A format Android cannot decode (e.g. SVG, TIFF) shows [unsupported].
 */
@Composable
internal fun ImagePage(
    media: LocalMedia,
    active: Boolean,
    onTap: () -> Unit,
    onZoomed: (Boolean) -> Unit,
    unsupported: @Composable () -> Unit,
) {
    var scale by rememberSaveable(media.path) { mutableFloatStateOf(1f) }
    var focusX by rememberSaveable(media.path) { mutableFloatStateOf(0.5f) }
    var focusY by rememberSaveable(media.path) { mutableFloatStateOf(0.5f) }
    LaunchedEffect(scale > 1f, active) { if (active) onZoomed(scale > 1f) }
    DisposableEffect(media.path) { onDispose { onZoomed(false) } }
    BoxWithConstraints(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        val density = LocalDensity.current
        val widthPx = with(density) { maxWidth.roundToPx() }.coerceAtLeast(1)
        val heightPx = with(density) { maxHeight.roundToPx() }.coerceAtLeast(1)
        val decoded by produceState<Decoded>(Decoded.Pending, media.path, widthPx, heightPx) {
            value = decodeImage(media.path, widthPx, heightPx)?.let { Decoded.Bitmap(it) } ?: Decoded.Unsupported
        }
        when (val result = decoded) {
            Decoded.Pending -> CircularProgressIndicator()
            Decoded.Unsupported -> unsupported()
            is Decoded.Bitmap -> ZoomableImage(
                result.image, widthPx.toFloat(), heightPx.toFloat(), scale, Offset(focusX, focusY), onTap,
            ) { zoom, focus -> scale = zoom; focusX = focus.x; focusY = focus.y }
        }
    }
}

@Composable
private fun ZoomableImage(
    image: ImageBitmap,
    pageWidth: Float,
    pageHeight: Float,
    scale: Float,
    focus: Offset,
    onTap: () -> Unit,
    onTransform: (Float, Offset) -> Unit,
) {
    val geometry = MediaGeometry.fit(image.width, image.height, pageWidth, pageHeight)
    val offset = geometry.offset(scale, focus)
    val transform = rememberTransformableState { zoomChange, panChange, _ ->
        val zoom = (scale * zoomChange).coerceIn(1f, MAX_ZOOM)
        val moved = geometry.clamp(offset * (zoom / scale) + panChange, zoom)
        onTransform(zoom, if (zoom == 1f) Offset(0.5f, 0.5f) else geometry.focus(zoom, moved))
    }
    Image(
        bitmap = image,
        contentDescription = null,
        contentScale = ContentScale.Fit,
        modifier = Modifier
            .fillMaxSize()
            .pointerInput(image, scale, focus) {
                detectTapGestures(
                    onTap = { onTap() },
                    onDoubleTap = {
                        if (scale > 1f) {
                            onTransform(1f, Offset(0.5f, 0.5f))
                        } else {
                            onTransform(DOUBLE_TAP_ZOOM, Offset(0.5f, 0.5f))
                        }
                    },
                )
            }
            // One finger pages while the picture is not zoomed (foundation 1.11 `canPan`).
            .transformable(transform, canPan = { scale > 1f })
            .graphicsLayer {
                scaleX = scale
                scaleY = scale
                translationX = offset.x
                translationY = offset.y
            },
    )
}

/**
 * Decodes [path] no larger than the page (EXIF orientation applied by ImageDecoder); `null` when
 * the format is not supported or the file cannot be read.
 */
private suspend fun decodeImage(path: String, maxWidth: Int, maxHeight: Int): ImageBitmap? =
    withContext(Dispatchers.IO) {
        try {
            ImageDecoder.decodeBitmap(ImageDecoder.createSource(File(path))) { decoder, info, _ ->
                val width = info.size.width
                val height = info.size.height
                val factor = maxOf(width.toFloat() / maxWidth, height.toFloat() / maxHeight)
                if (factor > 1f) {
                    decoder.setTargetSize(
                        (width / factor).toInt().coerceAtLeast(1),
                        (height / factor).toInt().coerceAtLeast(1),
                    )
                }
                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
            }.asImageBitmap()
        } catch (e: IOException) {
            null
        } catch (e: IllegalArgumentException) {
            null
        } catch (e: OutOfMemoryError) {
            null
        }
    }
