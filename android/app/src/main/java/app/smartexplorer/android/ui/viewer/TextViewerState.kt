package app.smartexplorer.android.ui.viewer

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.smartexplorer.android.api.MdDocumentDto
import app.smartexplorer.android.api.StyledLineDto
import app.smartexplorer.android.api.TextApi
import app.smartexplorer.android.api.TextFound
import app.smartexplorer.android.api.TextOpened
import app.smartexplorer.android.api.TextStatus
import app.smartexplorer.android.api.ThemeColorsDto
import app.smartexplorer.android.core.CoreException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** What the text view shows: a file, optionally at a line with a marked range (a search hit). */
internal data class TextViewRequest(
    val location: String,
    val name: String,
    /** Zero-based line to show first. */
    val line: Long = 0,
    /** UTF-16 range in [line] to mark (`start == end`: none). */
    val start: Int = 0,
    val end: Int = 0,
) {
    /** Local paths are opened in place, everything else is loaded through the core (api.md §2). */
    val local: Boolean
        get() = location.startsWith("/")
}

/**
 * State of one text view (api.md §4.15): the open document, its index status, the styled lines
 * loaded page by page (at most [MAX_PAGES] pages stay), language, theme, wrap and the current find
 * match. The core document is reopened once when it was closed meanwhile (`not_found`).
 */
internal class TextViewerState(private val scope: CoroutineScope, val request: TextViewRequest) {
    var opened by mutableStateOf<TextOpened?>(null)
        private set
    var status by mutableStateOf<TextStatus?>(null)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var known by mutableLongStateOf(0)
        private set
    var colors by mutableStateOf<ThemeColorsDto?>(null)
        private set

    /** Characters of the longest line loaded so far (width of the view without wrapping). */
    var longest by mutableIntStateOf(0)
        private set

    /** Language chosen by the person; `null` = the detected one. */
    var language by mutableStateOf<String?>(null)
        private set
    var shownLanguage by mutableStateOf("")
        private set
    var dark by mutableStateOf(false)
        private set
    var wrap by mutableStateOf(false)

    /** Markdown: rendered (true) or source (false). */
    var rendered by mutableStateOf(true)
    var markdown by mutableStateOf<MdDocumentDto?>(null)
        private set
    var markdownError by mutableStateOf<String?>(null)
        private set

    /** The current match of "Suchen" (or the search hit the view was opened at). */
    var match by mutableStateOf<TextFound?>(
        if (request.end > request.start) TextFound(true, request.line, request.start, request.end) else null,
    )
        private set

    private val pages = mutableStateMapOf<Long, List<StyledLineDto>>()
    private val loading = HashSet<Long>()
    private val used = LinkedHashSet<Long>()
    private var generation = 0

    /** Opens the document and follows its index until it is complete. */
    fun start(dark: Boolean) {
        this.dark = dark
        scope.launch {
            if (!reopen()) return@launch
            while (status?.complete != true) {
                delay(STATUS_INTERVAL_MS)
                val id = opened?.docId ?: return@launch
                try {
                    val fresh = TextApi.status(id)
                    status = fresh
                    known = maxOf(known, fresh.lines)
                    if (fresh.error != null) return@launch
                } catch (e: CoreException) {
                    if (e.kind != "not_found" || !reopen()) return@launch
                }
            }
        }
    }

    private suspend fun reopen(): Boolean = try {
        val doc = TextApi.open(request.location)
        opened = doc
        known = maxOf(known, doc.lines)
        shownLanguage = doc.language
        error = null
        true
    } catch (e: CoreException) {
        error = e.message ?: e.kind
        false
    }

    /** The styled line [index], `null` while its page is not loaded ([ensure] loads it). */
    fun line(index: Long): StyledLineDto? {
        val page = index / PAGE_LINES
        return pages[page]?.getOrNull((index - page * PAGE_LINES).toInt())
    }

    /** Loads the pages of lines [first]..[last] and the page on each side (shown lines first). */
    fun ensure(first: Long, last: Long) {
        if (last < first) return
        val from = first / PAGE_LINES
        val to = last / PAGE_LINES
        for (page in from..to) touch(page)
        for (page in listOf(from - 1, to + 1)) if (page >= 0 && page * PAGE_LINES < known) touch(page)
    }

    private fun touch(page: Long) {
        if (pages.containsKey(page)) {
            used.remove(page)
            used.add(page)
        } else {
            load(page)
        }
    }

    private fun load(page: Long) {
        val id = opened?.docId ?: return
        if (!loading.add(page)) return
        val wanted = generation
        scope.launch {
            try {
                val answer = try {
                    TextApi.lines(id, page * PAGE_LINES, PAGE_LINES.toInt(), dark, language)
                } catch (e: CoreException) {
                    if (e.kind != "not_found" || !reopen()) throw e
                    TextApi.lines(opened?.docId ?: return@launch, page * PAGE_LINES, PAGE_LINES.toInt(), dark, language)
                }
                if (wanted != generation) return@launch
                colors = answer.colors
                shownLanguage = answer.language
                known = maxOf(known, answer.known, page * PAGE_LINES + answer.lines.size)
                pages[page] = answer.lines
                longest = maxOf(longest, answer.lines.maxOfOrNull { it.text.length } ?: 0)
                used.add(page)
                while (used.size > MAX_PAGES) {
                    val oldest = used.first()
                    used.remove(oldest)
                    pages.remove(oldest)
                }
            } catch (e: CoreException) {
                error = e.message ?: e.kind
            } finally {
                loading.remove(page)
            }
        }
    }

    /** New theme or language: every page is styled again. */
    private fun restyle() {
        generation++
        pages.clear()
        used.clear()
        loading.clear()
        markdown = null
    }

    fun setTheme(dark: Boolean) {
        if (dark == this.dark) return
        this.dark = dark
        restyle()
    }

    fun chooseLanguage(name: String?) {
        if (name == language) return
        language = name
        restyle()
    }

    /** Renders the Markdown model once (again after a theme change). */
    fun loadMarkdown() {
        val id = opened?.docId ?: return
        if (markdown != null || markdownError != null) return
        val wanted = generation
        scope.launch {
            try {
                val doc = TextApi.markdown(id, dark)
                if (wanted == generation) markdown = doc
            } catch (e: CoreException) {
                markdownError = e.message ?: e.kind
                rendered = false
            }
        }
    }

    /**
     * Next (or previous) match of [pattern] after (before) the current one; the result becomes
     * [match]. Returns the line to scroll to, or `null` when nothing was found.
     */
    suspend fun find(pattern: String, regex: Boolean, matchCase: Boolean, backward: Boolean, fromTop: Long): TextFound? {
        val id = opened?.docId ?: return null
        val current = match
        val (line, column) = when {
            current == null -> fromTop to 0
            backward -> current.line to current.start
            else -> current.line to current.end.coerceAtLeast(current.start + 1)
        }
        val found = try {
            TextApi.find(id, pattern, regex, matchCase, line, column, backward)
        } catch (e: CoreException) {
            error = e.message ?: e.kind
            return null
        }
        error = null
        match = found.takeIf { it.found }
        return match
    }

    /** A new search text starts from the top of what is shown. */
    fun clearMatch() {
        match = null
    }

    /** The view closed: the core document goes (outside [scope], which may be cancelled). */
    fun close() {
        val id = opened?.docId ?: return
        detached.launch {
            try {
                TextApi.close(id)
            } catch (e: CoreException) {
                // Already closed.
            }
        }
    }

    companion object {
        const val PAGE_LINES = 200L
        const val MAX_PAGES = 40
        private const val STATUS_INTERVAL_MS = 300L
        private val detached = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    }
}
