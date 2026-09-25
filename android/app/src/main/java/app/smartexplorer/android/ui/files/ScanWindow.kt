package app.smartexplorer.android.ui.files

import android.os.SystemClock
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.ScanView
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.core.FilterSpec
import app.smartexplorer.android.core.SortSpec
import app.smartexplorer.android.core.TaskInfo
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * Recursive scan of one tab (spec F5) shown through `scan.view` windows of [PAGE] rows: only the
 * pages around the visible rows are loaded; while the scan runs the view is polled at most once
 * per second (`sinceRevision` avoids transferring unchanged windows).
 */
internal class ScanWindow(
    private val scope: CoroutineScope,
    /** Folder the scan started in (base for filtered copies). */
    val root: String,
    val filter: FilterSpec?,
    /** Scanned place is local (thumbnails, open without download). */
    val local: Boolean,
    private val showHidden: Boolean,
    private val sort: () -> SortSpec,
    private val collapsed: () -> Set<String>,
) {
    private enum class Wake { Missing, Force }

    /** Filter for copying from this view: hidden entries count only when the view showed them. */
    val transferFilter: FilterSpec?
        get() = filter?.let { if (showHidden && !it.hidden) it.copy(hidden = true) else it }

    var taskId by mutableStateOf<String?>(null)
        private set

    /** Rows of the tree view (after collapsing). */
    var total by mutableIntStateOf(0)
        private set
    var matches by mutableLongStateOf(0)
        private set
    var scanned by mutableLongStateOf(0)
        private set
    var truncated by mutableStateOf(false)
        private set
    var issues by mutableIntStateOf(0)
        private set
    var running by mutableStateOf(true)
        private set

    /** The scan could not start or its result is gone; the tab offers a new scan. */
    var error by mutableStateOf<String?>(null)
        private set

    private val pages = mutableStateMapOf<Int, List<Entry>>()
    private val wake = Channel<Wake>(Channel.CONFLATED)
    private var revision: Long? = null
    private var firstVisible = 0
    private var lastVisible = 0
    private var job: Job? = null

    fun start() {
        job = scope.launch {
            val id = try {
                FilesApi.scanStart(root, filter, showHidden)
            } catch (e: CoreException) {
                error = e.message ?: e.kind
                running = false
                return@launch
            }
            taskId = id
            loop(id)
        }
    }

    /** Stops polling and cancels a still running scan. */
    fun stop() {
        job?.cancel()
        val id = taskId ?: return
        if (running) {
            scope.launch {
                try {
                    FilesApi.cancelTask(id)
                } catch (e: CoreException) {
                    // The scan ended meanwhile (not_found / already done): nothing left to stop.
                }
            }
        }
    }

    /** [Stopp]: ends the scan; the matches found so far stay visible. */
    suspend fun cancelScan() {
        taskId?.let { FilesApi.cancelTask(it) }
    }

    fun entryAt(index: Int): Entry? = pages[index / PAGE]?.getOrNull(index % PAGE)

    /** Entries of the loaded windows (for "Auswahl umkehren"). */
    fun loadedEntries(): List<Entry> = pages.keys.sorted().flatMap { pages[it].orEmpty() }

    /** Called with the visible row range; loads missing windows. */
    fun onVisible(first: Int, last: Int) {
        firstVisible = first.coerceAtLeast(0)
        lastVisible = last.coerceAtLeast(firstVisible)
        if (neededPages().any { it !in pages }) wake.trySend(Wake.Missing)
    }

    /** Collapse state or sort order changed: reload the visible windows. */
    fun invalidate() {
        wake.trySend(Wake.Force)
    }

    /** Every row of the current tree view, or `null` when there are more than [max]. */
    suspend fun fetchAll(max: Int): List<Entry>? {
        val id = taskId ?: return emptyList()
        if (total > max) return null
        val rows = ArrayList<Entry>(total)
        var offset = 0
        while (true) {
            val view = FilesApi.scanView(id, sort(), collapsed(), offset, FilesApi.MAX_WINDOW, null)
            rows += view.entries
            offset += view.entries.size
            if (view.entries.isEmpty() || offset >= view.visibleTotal) return rows
            if (offset > max) return null
        }
    }

    private suspend fun loop(id: String) {
        var lastPoll = 0L
        var wakeReason: Wake? = Wake.Force
        while (true) {
            try {
                when (wakeReason) {
                    Wake.Missing -> fillMissing(id)
                    Wake.Force -> refresh(id, force = true)
                    null -> refresh(id, force = false)
                }
            } catch (e: CoreException) {
                // A cleared or unknown task cannot be viewed any more.
                if (e.kind == "not_found") {
                    error = "Das Scan-Ergebnis ist nicht mehr verfügbar."
                    running = false
                    return
                }
                error = e.message ?: e.kind
            }
            if (wakeReason != Wake.Missing) lastPoll = SystemClock.elapsedRealtime()
            // Normally known from task events; ask directly if none arrived yet.
            val task = Core.tasks.value.firstOrNull { it.id == id } ?: fetchTask(id)
            if (running && task != null && !task.isActive) {
                running = false
                // Final state of the finished scan.
                wakeReason = null
                continue
            }
            wakeReason = if (running) {
                val wait = (POLL_MS - (SystemClock.elapsedRealtime() - lastPoll)).coerceAtLeast(0)
                withTimeoutOrNull(wait) { wake.receive() }
            } else {
                wake.receive()
            }
        }
    }

    private suspend fun fetchTask(id: String): TaskInfo? = try {
        FilesApi.taskGet(id)
    } catch (e: CoreException) {
        null
    }

    private fun neededPages(): IntRange = (firstVisible / PAGE)..(lastVisible / PAGE)

    private suspend fun refresh(id: String, force: Boolean) {
        val needed = neededPages()
        val since = if (force || pages.isEmpty()) null else revision
        val head = FilesApi.scanView(id, sort(), collapsed(), needed.first * PAGE, PAGE, since)
        if (head.unchanged) {
            fillMissing(id)
            return
        }
        pages.clear()
        apply(head)
        pages[needed.first] = head.entries
        fillMissing(id)
    }

    private suspend fun fillMissing(id: String) {
        for (page in neededPages()) {
            if (page in pages || page * PAGE >= total && pages.isNotEmpty()) continue
            val view = FilesApi.scanView(id, sort(), collapsed(), page * PAGE, PAGE, null)
            if (view.revision != revision) {
                // The tree changed since the other windows were loaded.
                pages.clear()
                apply(view)
            }
            pages[page] = view.entries
        }
    }

    private fun apply(view: ScanView) {
        revision = view.revision
        total = view.visibleTotal
        matches = view.matches
        scanned = view.scanned
        truncated = view.truncated
        issues = view.issues
        error = null
    }

    companion object {
        const val PAGE = 300
        private const val POLL_MS = 1_000L
    }
}
