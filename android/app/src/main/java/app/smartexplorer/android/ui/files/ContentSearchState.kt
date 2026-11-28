package app.smartexplorer.android.ui.files

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.smartexplorer.android.api.ContentSearchQuery
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.ReplaceHit
import app.smartexplorer.android.api.TextApi
import app.smartexplorer.android.core.CoreException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * "In Dateien suchen" (api.md §4.14): the fields of Notepad++'s "Find in Files" plus the two
 * folder-path regex fields, the running search and its results, and "In Dateien ersetzen"
 * ([replace], see ContentSearchReplace.kt). Kept in the Files view model, so the form, the last
 * results and a running replacement survive tab switches, rotation and "Zurück".
 */
internal class ContentSearchState(private val scope: CoroutineScope) {
    /** Folder searched (a location of §2, local or remote). */
    var location by mutableStateOf<String?>(null)
        private set
    var pattern by mutableStateOf("")
    /** "Ersetzen durch"; the syntax follows [mode] (Notepad++). */
    var replacement by mutableStateOf("")
    /** `normal|extended|regex` */
    var mode by mutableStateOf("normal")
    var matchCase by mutableStateOf(false)
    var wholeWord by mutableStateOf(false)
    var dotMatchesNewline by mutableStateOf(false)
    var filters by mutableStateOf("")
    var folderInclude by mutableStateOf("")
    var folderExclude by mutableStateOf("")
    var subfolders by mutableStateOf(true)
    var hidden by mutableStateOf(false)
    var filesOnly by mutableStateOf(false)
    /** "Ignorierdateien beachten (.gitignore)"; off as in Notepad++. */
    var respectIgnoreFiles by mutableStateOf(false)
    var contextLines by mutableStateOf("0")
    var maxHits by mutableStateOf("")

    var results by mutableStateOf<ContentResults?>(null)
        private set

    /** Bumped on every applied poll answer, so the list recomposes. */
    var version by mutableIntStateOf(0)
        private set
    var taskId by mutableStateOf<String?>(null)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var collapsed by mutableStateOf<Set<String>>(emptySet())
        private set

    /** Keys of the selected result rows (`f:<rel>`, `l:<rel>:<index>`, `s`, `s:<index>`). */
    var selected by mutableStateOf<Set<String>>(emptySet())
        private set

    /** Why the last replacement command did not start. */
    var replaceError by mutableStateOf<String?>(null)
        private set

    /** Files a finished replacement changed leave the result they came from. */
    val replace = ContentReplaceFlow(scope) { changed ->
        results?.dropFiles(changed)
        selected = emptySet()
        version++
    }

    val running: Boolean
        get() = taskId != null

    private var job: Job? = null

    /** Opened from the folder menu of [folder]; the form keeps its values. */
    fun openAt(folder: String) {
        if (folder != location && !running) {
            results = null
            collapsed = emptySet()
        }
        location = folder
        error = null
    }

    fun query(): ContentSearchQuery = ContentSearchQuery(
        pattern = pattern,
        mode = mode,
        matchCase = matchCase,
        wholeWord = wholeWord && mode != "regex",
        dotMatchesNewline = dotMatchesNewline && mode == "regex",
        filters = filters.trim(),
        folderInclude = folderInclude.trim().ifEmpty { null },
        folderExclude = folderExclude.trim().ifEmpty { null },
        subfolders = subfolders,
        hidden = hidden,
        contextLines = contextLines.trim().toIntOrNull()?.coerceAtLeast(0) ?: 0,
        maxHits = maxHits.trim().toLongOrNull()?.coerceAtLeast(0) ?: 0,
        filesOnly = filesOnly,
        respectIgnoreFiles = respectIgnoreFiles,
    )

    /** [Alle suchen]: an invalid pattern is reported at once, before any task. */
    fun start() {
        val folder = location ?: return
        if (pattern.isEmpty()) {
            error = "Bitte einen Suchtext eingeben."
            return
        }
        job?.cancel()
        val query = query()
        job = scope.launch {
            val previous = taskId
            if (previous != null) cancelQuietly(previous)
            val id = try {
                TextApi.searchStart(folder, query)
            } catch (e: CoreException) {
                error = e.message ?: e.kind
                return@launch
            }
            error = null
            collapsed = emptySet()
            selected = emptySet()
            val fresh = ContentResults(query, folder)
            results = fresh
            taskId = id
            try {
                poll(id, fresh)
            } finally {
                if (taskId == id) taskId = null
                withContext(NonCancellable) {
                    try {
                        // The results live here; the finished task need not stay in the task list.
                        FilesApi.clearFinishedTasks(listOf(id))
                    } catch (e: CoreException) {
                        // Already removed.
                    }
                }
            }
        }
    }

    private suspend fun poll(id: String, target: ContentResults) {
        var since = 0L
        while (true) {
            val answer = try {
                TextApi.searchPoll(id, since)
            } catch (e: CoreException) {
                error = e.message ?: e.kind
                return
            }
            target.apply(answer)
            since = answer.next
            version++
            if (answer.done) return
            // A full answer means more are waiting: fetch them at once.
            if (answer.events.size < POLL_FULL) delay(POLL_INTERVAL_MS)
        }
    }

    /**
     * [Abbrechen]: the search ends as canceled; the hits found so far stay. Sent outside [scope],
     * which is already cancelled when the view model is cleared.
     */
    fun cancel() {
        val id = taskId ?: return
        detached.launch { cancelQuietly(id) }
    }

    /**
     * The view model is cleared: a running search stops. "Zurück" keeps it running (its results stay
     * here and show again when the page is opened).
     */
    fun closed() {
        cancel()
        replace.closed()
    }

    /** [In Dateien ersetzen]: every match below the folder, after the dry run and the confirmation. */
    fun replaceAll() {
        val folder = location ?: return
        if (pattern.isEmpty()) {
            replaceError = "Bitte einen Suchtext eingeben."
            return
        }
        replaceError = null
        replace.request(ReplaceJob(folder, query(), replacement, hits = null))
    }

    /** "Ersetzen…" for the selected rows (the confirmation asks "Ersetzen durch" first). */
    fun replaceSelected() = replaceHits { it.hitRefs(selected) }

    /** "Alle verbleibenden ersetzen…": every hit line still in the result. */
    fun replaceRemaining() = replaceHits { it.allHitRefs() }

    private fun replaceHits(pick: (ContentResults) -> List<ReplaceHit>) {
        val current = results
        replaceError = when {
            current == null -> "Es gibt kein Suchergebnis."
            running -> "Die Suche läuft noch."
            !current.replaceable -> "Diese Treffer lassen sich nicht ersetzen (Suche fehlgeschlagen oder nur Dateinamen)."
            else -> null
        }
        if (current == null || replaceError != null) return
        val hits = pick(current)
        if (hits.isEmpty()) {
            replaceError = "Die Auswahl enthält keine Trefferzeile."
            return
        }
        replace.ask(ReplaceJob(current.location, current.query, replacement, hits))
    }

    /** Long press on a row: into or out of the selection. */
    fun toggleSelected(key: String) {
        selected = if (key in selected) selected - key else selected + key
    }

    fun clearSelection() {
        selected = emptySet()
    }

    /** "Aus dem Ergebnis entfernen": the selected rows are never replaced. */
    fun dismissSelected() {
        val current = results ?: return
        if (current.dismiss(selected)) version++
        selected = emptySet()
    }

    fun toggle(key: String) {
        collapsed = if (key in collapsed) collapsed - key else collapsed + key
    }

    fun collapseAll() {
        val current = results ?: return
        collapsed = current.files.map { it.rel }.toSet() + SKIPPED_KEY
    }

    fun expandAll() {
        collapsed = emptySet()
    }

    private suspend fun cancelQuietly(id: String) {
        try {
            TextApi.searchCancel(id)
        } catch (e: CoreException) {
            // Already finished or gone.
        }
    }

    private companion object {
        val detached = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        const val POLL_INTERVAL_MS = 250L
        const val POLL_FULL = 2000
    }
}
