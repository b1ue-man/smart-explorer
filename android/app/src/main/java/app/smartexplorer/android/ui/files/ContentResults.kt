package app.smartexplorer.android.ui.files

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import app.smartexplorer.android.api.ContentLine
import app.smartexplorer.android.api.ContentPoll
import app.smartexplorer.android.api.ContentSearchQuery

// Result list of "In Dateien suchen" (api.md §4.14) like Notepad++'s search results: a head line
// `Suche „x“ (N Treffer in M Dateien von K durchsuchten)`, one foldable row per file with its hit
// count, one row per hit line `Zeile 12: …` with the match marked, then the skipped files.

/** One file with its hit and context lines in the order the core sent them. */
internal data class ResultFile(val rel: String, val location: String, val lines: List<ContentLine>) {
    /** Hit lines (context lines do not count). */
    val hits: Int
        get() = lines.count { !it.context }

    /** First hit line (for "open at the first hit"). */
    val firstHit: ContentLine?
        get() = lines.firstOrNull { !it.context }
}

/** A file or folder the search did not read, with the German reason. */
internal data class SkippedFile(val rel: String, val location: String, val reason: String)

/** One row of the result list. */
internal sealed interface ResultRow {
    val key: String

    data class File(val file: ResultFile, val expanded: Boolean) : ResultRow {
        override val key: String get() = "f:${file.rel}"
    }

    data class Line(val file: ResultFile, val line: ContentLine, val index: Int) : ResultRow {
        override val key: String get() = "l:${file.rel}:$index"
    }

    data class SkippedHeader(val count: Int, val expanded: Boolean) : ResultRow {
        override val key: String get() = "s"
    }

    data class Skipped(val skipped: SkippedFile, val index: Int) : ResultRow {
        override val key: String get() = "s:$index"
    }
}

/** Key under which the skipped files fold in [ContentResults.rows]. */
internal const val SKIPPED_KEY = "\u0000skipped"

/** The results of one search, built from the `search.content.poll` answers. */
internal class ContentResults(val query: ContentSearchQuery) {
    private val order = ArrayList<String>()
    private val lines = HashMap<String, MutableList<ContentLine>>()
    private val locations = HashMap<String, String>()
    private val skippedFiles = ArrayList<SkippedFile>()

    /** Files whose content was searched (the core's count). */
    var searched = 0L
        private set
    var done = false
        private set
    var canceled = false
        private set
    var error: String? = null
        private set

    /** The search stopped at "Höchstzahl Treffer". */
    var limited = false
        private set

    val files: List<ResultFile>
        get() = order.map { ResultFile(it, locations[it].orEmpty(), lines[it].orEmpty()) }

    val skipped: List<SkippedFile>
        get() = skippedFiles

    /** Hit lines received so far. */
    val hits: Int
        get() = lines.values.sumOf { file -> file.count { !it.context } }

    fun apply(poll: ContentPoll) {
        for (event in poll.events) {
            when (event.type) {
                "hits" -> {
                    val list = lines.getOrPut(event.rel) {
                        order += event.rel
                        locations[event.rel] = event.location
                        ArrayList()
                    }
                    list += event.lines
                }
                "skipped" -> skippedFiles += SkippedFile(event.rel, event.location, skipReason(event.reason, event.detail))
            }
        }
        searched = maxOf(searched, poll.summary?.filesSearched ?: poll.files)
        if (poll.done) done = true
        if (poll.canceled) canceled = true
        if (poll.error != null) error = poll.error
        if (poll.summary?.limited == true) limited = true
    }

    /** `Suche „x“ (3 Treffer in 2 Dateien von 10 durchsuchten) [Normal: Groß/Klein]`. */
    fun header(): String {
        val fileCount = order.size
        val fileWord = if (fileCount == 1) "Datei" else "Dateien"
        return "Suche „${query.pattern}“ ($hits Treffer in $fileCount $fileWord von $searched durchsuchten) ${modeLabel(query)}"
    }

    /** Rows as shown; files whose rel is in [collapsed] show only their head row. */
    fun rows(collapsed: Set<String>): List<ResultRow> {
        val out = ArrayList<ResultRow>()
        for (file in files) {
            val expanded = file.rel !in collapsed
            out += ResultRow.File(file, expanded)
            if (expanded) file.lines.forEachIndexed { index, line -> out += ResultRow.Line(file, line, index) }
        }
        if (skippedFiles.isNotEmpty()) {
            val expanded = SKIPPED_KEY !in collapsed
            out += ResultRow.SkippedHeader(skippedFiles.size, expanded)
            if (expanded) skippedFiles.forEachIndexed { index, file -> out += ResultRow.Skipped(file, index) }
        }
        return out
    }
}

/** The bracket after the head line, as Notepad++ shows the search mode and its options. */
internal fun modeLabel(query: ContentSearchQuery): String {
    val mode = when (query.mode) {
        "extended" -> "Erweitert"
        "regex" -> "Regex"
        else -> "Normal"
    }
    val options = buildList {
        if (query.matchCase) add("Groß/Klein")
        if (query.wholeWord && query.mode != "regex") add("Wort")
        if (query.dotMatchesNewline && query.mode == "regex") add(". = Zeilenumbruch")
    }
    return if (options.isEmpty()) "[$mode]" else "[$mode: ${options.joinToString("/")}]"
}

internal fun skipReason(reason: String?, detail: String?): String = when (reason) {
    "binary" -> "Binärdatei (keine Textdatei)"
    "unreadable" -> "nicht lesbar" + (detail?.let { ": $it" } ?: "")
    "unlistable" -> "Ordner nicht lesbar" + (detail?.let { ": $it" } ?: "")
    else -> detail ?: "ausgelassen"
}

/** `Zeile 12: text` with every match of [line] in [mark]; a cut line ends with `…`. */
internal fun hitText(line: ContentLine, mark: SpanStyle): AnnotatedString = buildAnnotatedString {
    val prefix = "Zeile ${line.line}: "
    append(prefix)
    append(line.text)
    if (line.truncated) append("…")
    for (range in line.ranges) {
        val start = range.getOrNull(0) ?: continue
        val end = range.getOrNull(1) ?: continue
        val from = start.coerceIn(0, line.text.length)
        val to = end.coerceIn(from, line.text.length)
        if (to > from) addStyle(mark, prefix.length + from, prefix.length + to)
    }
}
