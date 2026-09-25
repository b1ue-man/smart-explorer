package app.smartexplorer.android.ui.analytics

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.smartexplorer.android.api.AnalyzeChild
import app.smartexplorer.android.ui.common.Format

/** Largest number of children drawn as own rectangles; the rest share one "Weitere" tile. */
private const val MAX_TILES = 60

/** Tile colors by type (spec F19 "Farbe nach Typ"); fixed mid tones, readable in both themes. */
internal enum class TileKind(val label: String, val color: Color) {
    Folder("Ordner", Color(0xFF6B93D6)),
    Image("Bilder", Color(0xFF66BB6A)),
    Video("Videos", Color(0xFFE57373)),
    Audio("Audio", Color(0xFFBA68C8)),
    Archive("Archive", Color(0xFFFFB74D)),
    Document("Dokumente", Color(0xFF4DB6AC)),
    Other("Sonstige", Color(0xFF90A4AE)),
}

private val EXTENSIONS: Map<String, TileKind> = buildMap {
    listOf("jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "svg", "dng", "raw").forEach { put(it, TileKind.Image) }
    listOf("mp4", "mkv", "mov", "avi", "webm", "3gp", "m4v", "wmv").forEach { put(it, TileKind.Video) }
    listOf("mp3", "flac", "wav", "ogg", "opus", "m4a", "aac", "wma").forEach { put(it, TileKind.Audio) }
    listOf("zip", "7z", "rar", "tar", "gz", "tgz", "xz", "bz2", "zst", "apk", "iso").forEach { put(it, TileKind.Archive) }
    listOf("pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "txt", "md", "epub").forEach {
        put(it, TileKind.Document)
    }
}

internal fun tileKind(child: AnalyzeChild): TileKind {
    if (child.isDir) return TileKind.Folder
    val ext = child.name.substringAfterLast('.', "").lowercase()
    return EXTENSIONS[ext] ?: TileKind.Other
}

/** A drawn tile; [child] is `null` for the aggregated rest. */
private class Tile(val child: AnalyzeChild?, val label: String, val size: Long, val color: Color)

/**
 * Treemap of [children] (squarified, drawn on a Canvas). Tapping a folder tile calls [onOpen].
 * Labels are drawn only where they fit.
 */
@Composable
internal fun TreemapView(children: List<AnalyzeChild>, onOpen: (AnalyzeChild) -> Unit, modifier: Modifier = Modifier) {
    val tiles = remember(children) { tilesOf(children) }
    val currentOnOpen by rememberUpdatedState(onOpen)
    val measurer = rememberTextMeasurer()
    val border = MaterialTheme.colorScheme.surface
    Canvas(
        modifier
            .fillMaxWidth()
            .height(240.dp)
            .semantics { contentDescription = "Treemap der größten Einträge" }
            .pointerInput(tiles) {
                detectTapGestures(onTap = { offset ->
                    // Same pure layout as the drawing below, so hits match the tiles.
                    Treemap.squarify(tiles.map { it.size }, size.width.toFloat(), size.height.toFloat())
                        .firstOrNull { it.contains(offset.x, offset.y) }
                        ?.let { hit -> tiles[hit.index].child?.takeIf { it.isDir }?.let(currentOnOpen) }
                })
            },
    ) {
        val cells = Treemap.squarify(tiles.map { it.size }, size.width, size.height)
        val minLabelWidth = 56.dp.toPx()
        val minLabelHeight = 30.dp.toPx()
        val padding = 4.dp.toPx()
        val style = TextStyle(color = Color(0xDE000000), fontSize = 11.sp)
        cells.forEach { cell ->
            val tile = tiles[cell.index]
            val topLeft = Offset(cell.left, cell.top)
            val cellSize = Size(cell.width, cell.height)
            drawRect(color = tile.color, topLeft = topLeft, size = cellSize)
            drawRect(color = border, topLeft = topLeft, size = cellSize, style = Stroke(width = 1.dp.toPx()))
            if (cell.width >= minLabelWidth && cell.height >= minLabelHeight) {
                drawText(
                    measurer,
                    "${tile.label}\n${Format.size(tile.size)}",
                    topLeft = Offset(cell.left + padding, cell.top + padding),
                    style = style,
                    overflow = TextOverflow.Ellipsis,
                    maxLines = 2,
                    size = Size(cell.width - 2 * padding, cell.height - 2 * padding),
                )
            }
        }
    }
}

/** Color legend below the treemap (only kinds that occur). */
@Composable
internal fun TreemapLegend(children: List<AnalyzeChild>, modifier: Modifier = Modifier) {
    val kinds = remember(children) { children.map(::tileKind).distinct().sortedBy { it.ordinal } }
    Row(modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        kinds.forEach { kind ->
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                Box(Modifier.size(10.dp).background(kind.color, RoundedCornerShape(2.dp)))
                Text(kind.label, style = MaterialTheme.typography.labelSmall)
            }
        }
    }
}

private fun tilesOf(children: List<AnalyzeChild>): List<Tile> {
    val sorted = children.filter { it.size > 0 }.sortedByDescending { it.size }
    val shown = sorted.take(MAX_TILES).map { Tile(it, it.name, it.size, tileKind(it).color) }
    val rest = sorted.drop(MAX_TILES)
    if (rest.isEmpty()) return shown
    val label = "${rest.size} weitere"
    return shown + Tile(null, label, rest.sumOf { it.size }, TileKind.Other.color.copy(alpha = 0.6f))
}

/** Bar for one list row: share of [size] in [total]. */
internal fun shareOf(size: Long, total: Long): Float = if (total <= 0) 0f else (size.toDouble() / total).coerceIn(0.0, 1.0).toFloat()

/** Treemap with its color legend. */
@Composable
internal fun TreemapBlock(children: List<AnalyzeChild>, onOpen: (AnalyzeChild) -> Unit, modifier: Modifier = Modifier) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        TreemapView(children, onOpen)
        TreemapLegend(children)
    }
}
