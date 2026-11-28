package app.smartexplorer.android.ui.viewer

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.MdBlockDto
import app.smartexplorer.android.api.MdDocumentDto
import app.smartexplorer.android.api.MdRunDto
import app.smartexplorer.android.api.ThemeColorsDto
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.system.Thumbnails

/** How a rendered document reaches its links and pictures. */
internal class MdContext(
    /** `prefix` and `path` of `text.open`: relative links and pictures resolve against them. */
    val prefix: String,
    val path: String,
    val colors: ThemeColorsDto?,
    val onLink: (String) -> Unit,
)

/**
 * The Markdown model of the core (`text.markdown`) drawn with Compose: headings, inline styles,
 * links (http(s) in the browser, relative paths open in the text view), lists and tasks, quotes and
 * GitHub alerts, code blocks with the core's spans, tables (scroll sideways), rules, pictures through
 * the app's picture loading (else their alternative text), footnotes. Raw HTML is shown as text.
 */
@Composable
internal fun MarkdownView(document: MdDocumentDto, context: MdContext, modifier: Modifier = Modifier) {
    LazyColumn(modifier.fillMaxSize(), contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        itemsIndexed(document.blocks) { _, block -> MdBlock(block, context) }
    }
}

@Composable
private fun MdBlock(block: MdBlockDto, context: MdContext) {
    val typography = MaterialTheme.typography
    when (block.type) {
        "heading" -> Text(
            runsText(block.runs, context),
            style = when (block.level) {
                1 -> typography.headlineMedium
                2 -> typography.headlineSmall
                3 -> typography.titleLarge
                4 -> typography.titleMedium
                else -> typography.titleSmall
            },
        )
        "paragraph" -> Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(runsText(block.runs, context), style = typography.bodyLarge)
            // Inline pictures follow their paragraph.
            block.runs.filter { it.kind.type == "image" }.forEach { MdImage(it.kind.url, it.text, context) }
        }
        "list" -> Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            block.items.forEachIndexed { index, item ->
                Row {
                    val marker = when {
                        item.task == true -> "☑"
                        item.task == false -> "☐"
                        block.start != null -> "${block.start + index}."
                        else -> "•"
                    }
                    Text(marker, style = typography.bodyLarge, modifier = Modifier.width(32.dp))
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        item.blocks.forEach { MdBlock(it, context) }
                    }
                }
            }
        }
        "quote" -> Row(Modifier.height(IntrinsicSize.Min)) {
            Box(Modifier.width(4.dp).fillMaxHeight().background(quoteColor(block.kind)))
            Column(Modifier.padding(start = 12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                alertLabel(block.kind)?.let {
                    Text(it, style = typography.titleSmall, color = quoteColor(block.kind))
                }
                block.blocks.forEach { MdBlock(it, context) }
            }
        }
        "code" -> CodeBlock(block, context)
        "table" -> MdTable(block, context)
        "rule" -> HorizontalDivider()
        "image" -> MdImage(block.url, block.alt, context)
        "html" -> Text(
            block.text,
            style = typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        "footnote_definition" -> Row {
            Text("[${block.label}]", style = typography.bodySmall, modifier = Modifier.width(40.dp))
            Column(Modifier.weight(1f)) { block.blocks.forEach { MdBlock(it, context) } }
        }
        // A block type of a later core: its text, never nothing.
        else -> if (block.text.isNotEmpty()) Text(block.text) else block.blocks.forEach { MdBlock(it, context) }
    }
}

@Composable
private fun CodeBlock(block: MdBlockDto, context: MdContext) {
    val background = context.colors?.let { argbColor(it.background) } ?: MaterialTheme.colorScheme.surfaceVariant
    val style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace)
    Column(
        Modifier
            .fillMaxWidth()
            .background(background, RoundedCornerShape(6.dp))
            .horizontalScroll(rememberScrollState())
            .padding(10.dp),
    ) {
        block.lines.forEach { line -> Text(styledText(line), style = style, softWrap = false) }
    }
}

@Composable
private fun MdTable(block: MdBlockDto, context: MdContext) {
    val columns = maxOf(block.header.size, block.rows.maxOfOrNull { it.size } ?: 0)
    if (columns == 0) return
    // One width per column from its longest cell, so the rows line up.
    val widths = (0 until columns).map { column ->
        val longest = (listOf(block.header) + block.rows).maxOf { row -> row.getOrNull(column)?.sumOf { it.text.length } ?: 0 }
        (longest * 8 + 24).coerceIn(64, 320).dp
    }
    val border = MaterialTheme.colorScheme.outlineVariant
    Column(Modifier.horizontalScroll(rememberScrollState())) {
        (listOf(block.header) + block.rows).forEachIndexed { rowIndex, row ->
            Row {
                for (column in 0 until columns) {
                    val runs = row.getOrNull(column).orEmpty()
                    val base = runsText(runs, context)
                    Box(Modifier.width(widths[column]).border(0.5.dp, border).padding(6.dp)) {
                        Text(
                            base,
                            style = if (rowIndex == 0) MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.Bold) else MaterialTheme.typography.bodyMedium,
                        )
                    }
                }
            }
        }
    }
}

/** A picture of the document: files of the same connection through the app's loading, else the alternative text. */
@Composable
private fun MdImage(url: String, alt: String, context: MdContext) {
    val target = resolveLink(context.prefix, context.path, url)
    val px = with(LocalDensity.current) { 640.dp.roundToPx() }
    val bitmap by produceState<ImageBitmap?>(null, target, px) {
        val file = target as? LinkTarget.File ?: return@produceState
        value = try {
            val media = loadMedia(Entry(name = file.name, location = file.location, isDir = false), file.location.startsWith("/")) {}
            Thumbnails.load(media.path, 0, px)
        } catch (e: CoreException) {
            null
        }
    }
    val image = bitmap
    if (image != null) {
        Image(
            bitmap = image,
            contentDescription = alt.ifBlank { null },
            contentScale = ContentScale.Fit,
            modifier = Modifier.fillMaxWidth().heightIn(max = 480.dp),
        )
    } else {
        val label = buildAnnotatedString {
            val text = "Bild: ${alt.ifBlank { url }}"
            if (target is LinkTarget.None) {
                append(text)
            } else {
                withLink(LinkAnnotation.Clickable(url, linkStyles()) { context.onLink(url) }) { append(text) }
            }
        }
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun linkStyles(): TextLinkStyles =
    TextLinkStyles(SpanStyle(color = MaterialTheme.colorScheme.primary, textDecoration = TextDecoration.Underline))

/** Inline runs with their emphasis, code and link styles; links call [MdContext.onLink]. */
@Composable
private fun runsText(runs: List<MdRunDto>, context: MdContext): AnnotatedString {
    val links = linkStyles()
    val codeBackground = MaterialTheme.colorScheme.surfaceVariant
    return buildAnnotatedString {
        for (run in runs) {
            val text = when (run.kind.type) {
                "line_break" -> "\n"
                "image" -> continue
                "footnote_ref" -> "[${run.text}]"
                else -> run.text
            }
            val style = SpanStyle(
                fontWeight = if (run.strong) FontWeight.Bold else null,
                fontStyle = if (run.emphasis) FontStyle.Italic else null,
                textDecoration = if (run.strikethrough) TextDecoration.LineThrough else null,
                fontFamily = if (run.kind.type == "code" || run.kind.type == "html") FontFamily.Monospace else null,
                background = if (run.kind.type == "code") codeBackground else Color.Unspecified,
            )
            val link = run.link
            if (link != null) {
                withLink(LinkAnnotation.Clickable(link, links) { context.onLink(link) }) { withStyle(style) { append(text) } }
            } else {
                withStyle(style) { append(text) }
            }
        }
    }
}

@Composable
private fun quoteColor(kind: String?): Color = when (kind) {
    "note" -> MaterialTheme.colorScheme.primary
    "tip" -> Color(0xFF2E7D32)
    "important" -> MaterialTheme.colorScheme.tertiary
    "warning" -> Color(0xFFB26A00)
    "caution" -> MaterialTheme.colorScheme.error
    else -> MaterialTheme.colorScheme.outline
}

private fun alertLabel(kind: String?): String? = when (kind) {
    "note" -> "Hinweis"
    "tip" -> "Tipp"
    "important" -> "Wichtig"
    "warning" -> "Warnung"
    "caution" -> "Vorsicht"
    else -> null
}
