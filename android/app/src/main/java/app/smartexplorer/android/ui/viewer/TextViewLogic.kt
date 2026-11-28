package app.smartexplorer.android.ui.viewer

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import app.smartexplorer.android.api.StyledLineDto
import app.smartexplorer.android.api.StyledSpanDto
import java.net.URLDecoder

// The parts of the text view that need no Android: spans → AnnotatedString, "Gehe zu Zeile",
// Markdown link targets. The view knows no languages: the core sends spans and colors (api.md §4.15).

/** 32-bit ARGB of the core as a Compose color. */
internal fun argbColor(argb: Long): Color = Color(argb.toInt())

internal fun spanStyle(span: StyledSpanDto): SpanStyle = SpanStyle(
    color = argbColor(span.fg),
    background = span.bg?.let(::argbColor) ?: Color.Unspecified,
    fontWeight = if (span.bold) FontWeight.Bold else null,
    fontStyle = if (span.italic) FontStyle.Italic else null,
    textDecoration = if (span.underline) TextDecoration.Underline else null,
)

/**
 * [line] with its spans (UTF-16 ranges of the core, clamped to the text) and, when [match] is set,
 * the match of "Suchen" in [matchStyle] on top.
 */
internal fun styledText(line: StyledLineDto, match: IntRange? = null, matchStyle: SpanStyle? = null): AnnotatedString =
    buildAnnotatedString {
        append(line.text)
        val length = line.text.length
        for (span in line.spans) {
            val start = span.start.coerceIn(0, length)
            val end = span.end.coerceIn(start, length)
            if (end > start) addStyle(spanStyle(span), start, end)
        }
        if (match != null && matchStyle != null) {
            val start = match.first.coerceIn(0, length)
            val end = (match.last + 1).coerceIn(start, length)
            if (end > start) addStyle(matchStyle, start, end)
        }
    }

/** Answer of "Gehe zu Zeile". */
internal sealed interface GoToLine {
    /** Zero-based line index. */
    data class Line(val index: Long) : GoToLine

    data class Invalid(val message: String) : GoToLine
}

/**
 * The line typed into "Gehe zu Zeile" (1-based, as shown in the gutter) for a document with
 * [known] lines so far; [complete] once the index is built. Beyond the end of a complete document
 * the last line is named, as Notepad++ does.
 */
internal fun resolveGoToLine(input: String, known: Long, complete: Boolean): GoToLine {
    val number = input.trim().toLongOrNull() ?: return GoToLine.Invalid("Bitte eine Zeilennummer eingeben.")
    if (number < 1) return GoToLine.Invalid("Zeilennummern beginnen bei 1.")
    if (known <= 0) return GoToLine.Invalid("Die Datei wird noch gelesen.")
    return when {
        number <= known -> GoToLine.Line(number - 1)
        complete -> GoToLine.Invalid("Die Datei hat $known Zeilen.")
        else -> GoToLine.Invalid("Bisher $known Zeilen gelesen – die Datei wird noch gelesen.")
    }
}

/** Where a Markdown link or image points. */
internal sealed interface LinkTarget {
    /** `http(s)://`, `mailto:`: the browser or mail app. */
    data class External(val url: String) : LinkTarget

    /** A file next to the document (same connection): opens in the text view. */
    data class File(val location: String, val name: String) : LinkTarget

    /** `#abschnitt` inside the document, or nothing usable. */
    data object None : LinkTarget
}

private val EXTERNAL = Regex("^(https?|mailto):", RegexOption.IGNORE_CASE)
private val SCHEME = Regex("^[a-zA-Z][a-zA-Z0-9+.-]*:")

/**
 * Resolves [target] of a Markdown link in the document at `prefix + path` (the `prefix` and `path`
 * of `text.open`): relative paths against the document's folder, `/…` against the root of the same
 * connection; `..` never climbs above that root. Query and fragment are dropped, `%xx` decoded.
 */
internal fun resolveLink(prefix: String, path: String, target: String): LinkTarget {
    val raw = target.trim()
    if (raw.isEmpty() || raw.startsWith("#")) return LinkTarget.None
    if (EXTERNAL.containsMatchIn(raw)) return LinkTarget.External(raw)
    if (SCHEME.containsMatchIn(raw)) return LinkTarget.None
    val bare = raw.substringBefore('#').substringBefore('?')
    if (bare.isEmpty()) return LinkTarget.None
    val decoded = try {
        URLDecoder.decode(bare.replace("+", "%2B"), "UTF-8")
    } catch (e: IllegalArgumentException) {
        bare
    }
    val base = if (decoded.startsWith("/")) emptyList() else path.split('/').filter { it.isNotEmpty() }.dropLast(1)
    val parts = ArrayList(base)
    for (part in decoded.split('/')) {
        when (part) {
            "", "." -> Unit
            ".." -> if (parts.isNotEmpty()) parts.removeAt(parts.size - 1)
            else -> parts += part
        }
    }
    if (parts.isEmpty()) return LinkTarget.None
    return LinkTarget.File(prefix + "/" + parts.joinToString("/"), parts.last())
}
