package app.smartexplorer.android.ui.screen

import android.graphics.Bitmap
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.ScreenCursorEvent
import java.nio.ByteBuffer
import kotlin.math.roundToInt
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.filterIsInstance

/**
 * The host's pointer over the picture (plan 2026-11-14, package P6): shape and position of a host
 * that leaves the pointer out of the picture arrive as events, so a pointer movement does not wait
 * for capture, encoding, transport and decoding of a picture. The picture lies at ([left], [top])
 * with the size [width] x [height] (pixels); nothing is drawn until a visible pointer of the
 * session's shape is known, so a host that keeps the pointer in the picture shows it as before.
 * A phone has no pointer of its own between touches, so the host's position is drawn in the modes
 * viewing and controlling alike (a touch moves the host's pointer, which reports it back).
 */
@Composable
fun ScreenCursorOverlay(session: Long, left: Float, top: Float, width: Float, height: Float) {
    var shape by remember(session) { mutableStateOf<ScreenCursorEvent.Shape?>(null) }
    var image by remember(session) { mutableStateOf<ImageBitmap?>(null) }
    var position by remember(session) { mutableStateOf<ScreenCursorEvent.Position?>(null) }
    LaunchedEffect(session) {
        if (session == 0L) return@LaunchedEffect
        Core.events.filterIsInstance<ScreenCursorEvent>().filter { it.session == session }.collect { event ->
            when (event) {
                is ScreenCursorEvent.Shape -> { shape = event; image = event.toImage() }
                is ScreenCursorEvent.Position -> position = event
            }
        }
    }
    val known = shape
    val where = position
    val picture = image
    if (known == null || where == null || picture == null) return
    if (!where.visible || where.shapeId != known.id) return
    Canvas(Modifier.fillMaxSize()) { drawCursor(picture, known, where, left, top, width, height) }
}

private fun ScreenCursorEvent.Shape.toImage(): ImageBitmap {
    val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
    bitmap.copyPixelsFromBuffer(ByteBuffer.wrap(rgba))
    return bitmap.asImageBitmap()
}

/** Where the pointer's top-left corner lies and how large it is, for a picture at ([left], [top]). */
internal fun cursorRect(shape: ScreenCursorEvent.Shape, at: ScreenCursorEvent.Position, left: Float, top: Float, width: Float, height: Float): List<Int> {
    val scaleX = width / at.areaWidth
    val scaleY = height / at.areaHeight
    val x = left + at.x * scaleX - shape.hotX * shape.shrink * scaleX
    val y = top + at.y * scaleY - shape.hotY * shape.shrink * scaleY
    val w = (shape.width * shape.shrink * scaleX).roundToInt().coerceAtLeast(1)
    val h = (shape.height * shape.shrink * scaleY).roundToInt().coerceAtLeast(1)
    return listOf(x.roundToInt(), y.roundToInt(), w, h)
}

private fun DrawScope.drawCursor(
    image: ImageBitmap,
    shape: ScreenCursorEvent.Shape,
    at: ScreenCursorEvent.Position,
    left: Float,
    top: Float,
    width: Float,
    height: Float,
) {
    val (x, y, w, h) = cursorRect(shape, at, left, top, width, height)
    drawImage(
        image,
        srcOffset = IntOffset.Zero,
        srcSize = IntSize(image.width, image.height),
        dstOffset = IntOffset(x, y),
        dstSize = IntSize(w, h),
    )
}
