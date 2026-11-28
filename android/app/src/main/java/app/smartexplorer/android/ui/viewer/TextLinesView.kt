package app.smartexplorer.android.ui.viewer

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.max

/** Lines read ahead of and behind the visible ones. */
private const val AHEAD_LINES = 40

/**
 * The lines of a document with line numbers, in the theme colors of the core; the spans come
 * from the core, the view only paints them. Without wrapping the whole list scrolls sideways.
 * [jump] (zero-based line) scrolls there once the line is known and is then cleared.
 */
@Composable
internal fun TextLinesView(state: TextViewerState, jump: MutableState<Long?>) {
    val listState = rememberLazyListState()
    val colors = state.colors
    val background = colors?.let { argbColor(it.background) } ?: MaterialTheme.colorScheme.surface
    val foreground = colors?.let { argbColor(it.foreground) } ?: MaterialTheme.colorScheme.onSurface
    val gutter = colors?.let { argbColor(it.gutter) } ?: MaterialTheme.colorScheme.onSurfaceVariant
    val matchStyle = SpanStyle(background = colors?.let { argbColor(it.selection) } ?: MaterialTheme.colorScheme.tertiaryContainer)
    val style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace, color = foreground)
    val measurer = rememberTextMeasurer()
    val charWidth = remember(style) { measurer.measure("0", style).size.width.coerceAtLeast(1) }
    val count = state.known
    val digits = count.coerceAtLeast(1).toString().length
    val density = LocalDensity.current
    val gutterWidth = with(density) { (digits * charWidth).toDp() } + 12.dp

    LaunchedEffect(listState, state.opened?.docId, state.dark, state.language) {
        snapshotFlow { listState.firstVisibleItemIndex to listState.layoutInfo.visibleItemsInfo.size }
            .collect { (first, visible) ->
                val from = (first - AHEAD_LINES).coerceAtLeast(0).toLong()
                state.ensure(from, (first + visible + AHEAD_LINES).toLong())
            }
    }
    LaunchedEffect(jump.value, count) {
        val target = jump.value ?: return@LaunchedEffect
        if (target < count) {
            listState.scrollToItem((target - 3).coerceAtLeast(0).toInt())
            jump.value = null
        }
    }

    BoxWithConstraints(Modifier.fillMaxSize().background(background)) {
        val textWidth = with(density) { (state.longest * charWidth).toDp() }
        val contentWidth = if (state.wrap) maxWidth else max(maxWidth, gutterWidth + textWidth + 24.dp)
        val sideways = rememberScrollState()
        SelectionContainer(
            if (state.wrap) Modifier.fillMaxSize() else Modifier.fillMaxSize().horizontalScroll(sideways),
        ) {
            LazyColumn(state = listState, modifier = Modifier.width(contentWidth).fillMaxHeight()) {
                items(count.coerceAtMost(Int.MAX_VALUE.toLong()).toInt()) { index ->
                    val line = state.line(index.toLong())
                    val match = state.match?.takeIf { it.line == index.toLong() }
                    Row(Modifier.fillMaxWidth()) {
                        DisableSelection {
                            Text(
                                "${index + 1}",
                                style = style.copy(color = gutter),
                                textAlign = TextAlign.End,
                                softWrap = false,
                                modifier = Modifier.width(gutterWidth).padding(end = 8.dp),
                            )
                        }
                        Text(
                            line?.let { styledText(it, match?.let { found -> found.start until found.end }, matchStyle) }
                                ?: AnnotatedString(""),
                            style = style,
                            softWrap = state.wrap,
                            modifier = Modifier.weight(1f).padding(end = 12.dp),
                        )
                    }
                }
            }
        }
    }
}
