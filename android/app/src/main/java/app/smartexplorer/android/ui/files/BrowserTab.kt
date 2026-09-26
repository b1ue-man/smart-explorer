package app.smartexplorer.android.ui.files

import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.Listing
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.core.FilterSpec
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * One tab of the Files browser (spec F3): own place, history (back/forward), filter row,
 * recursive scan, collapse state, selection and list position.
 */
internal class BrowserTab(
    val id: Long,
    initialLocation: String,
    private val scope: CoroutineScope,
    private val options: () -> ViewOptions,
) {
    /** Requested place; replaced by the core's normalized location once listed. */
    var location by mutableStateOf(initialLocation)
        private set
    var listing by mutableStateOf<Listing?>(null)
        private set
    var loading by mutableStateOf(false)
        private set

    /** Pull-to-refresh in progress (not shown for ordinary navigation). */
    var refreshing by mutableStateOf(false)
        private set
    var error by mutableStateOf<CoreException?>(null)
        private set
    var scan by mutableStateOf<ScanWindow?>(null)
        private set
    var collapsed by mutableStateOf<Set<String>>(emptySet())
        private set

    /** View options changed while the tab was not displayed; reload when shown. */
    var stale = false

    val filter = FilterState()

    /** Selected entries by location (kept across scan windows). */
    val selection = mutableStateMapOf<String, Entry>()

    /** List position; replaced by [detachList] whenever the tab's pane leaves the screen. */
    var listState by mutableStateOf(LazyListState())
        private set

    private val back = mutableStateListOf<HistoryItem>()
    private val forward = mutableStateListOf<HistoryItem>()
    private var loadJob: Job? = null
    private var filterJob: Job? = null
    private var generation = 0

    val canGoForward: Boolean
        get() = forward.isNotEmpty()

    /** Back goes through the history, then to the parent folder. */
    val canGoBack: Boolean
        get() = back.isNotEmpty() || listing?.parent != null

    /** Where copies and new items go: the listed folder, unless it is read-only. */
    val writableLocation: String?
        get() = listing?.takeIf { !it.readOnly && error == null }?.location

    /** Opens [target] as a new history step. */
    fun open(target: String) {
        val current = here()
        if (current.location != target) back.add(current)
        forward.clear()
        navigate(target, HistoryItem(target))
    }

    fun goBack(): Boolean {
        if (back.isNotEmpty()) {
            val previous = back.removeAt(back.lastIndex)
            forward.add(here())
            navigate(previous.location, previous)
            return true
        }
        val parent = listing?.parent ?: return false
        forward.add(here())
        navigate(parent, HistoryItem(parent))
        return true
    }

    fun goForward() {
        if (forward.isEmpty()) return
        val next = forward.removeAt(forward.lastIndex)
        back.add(here())
        navigate(next.location, next)
    }

    /** Reloads the current place and keeps the list position. */
    fun reload() = load(scrollTo = null)

    fun pullRefresh() {
        refreshing = true
        load(scrollTo = null)
    }

    /** Filter text or criteria changed: apply after a short pause while typing. */
    fun filterChanged(immediate: Boolean = false) {
        filterJob?.cancel()
        filterJob = scope.launch {
            if (!immediate) delay(FILTER_DEBOUNCE_MS)
            load(scrollTo = HistoryItem(location))
        }
    }

    fun setRecursive(on: Boolean) {
        if (filter.recursive == on) return
        filter.recursive = on
        filter.open = true
        selection.clear()
        collapsed = emptySet()
        load(scrollTo = HistoryItem(location))
    }

    fun closeFilter() {
        val hadEffect = filter.spec() != null || filter.recursive
        filter.close()
        if (hadEffect) load(scrollTo = HistoryItem(location))
    }

    fun toggleCollapsed(location: String) {
        collapsed = if (location in collapsed) collapsed - location else collapsed + location
        scan?.invalidate()
    }

    /** View options changed; a new hidden-files setting needs a new scan, sorting only a new view. */
    fun optionsChanged(hiddenChanged: Boolean) {
        val window = scan
        if (window != null && !hiddenChanged) window.invalidate() else load(scrollTo = null)
    }

    /** Entries currently shown (flat listing or loaded scan windows). */
    fun shownEntries(): List<Entry> = scan?.loadedEntries() ?: listing?.entries.orEmpty()

    fun toggleSelection(entry: Entry) {
        if (selection.remove(entry.location) == null) selection[entry.location] = entry
    }

    /**
     * The pane left composition (tab switch, activity recreated): keep the position, but drop the
     * state object that still references the old layout tree and its activity.
     */
    fun detachList() {
        listState = LazyListState(listState.firstVisibleItemIndex, listState.firstVisibleItemScrollOffset)
    }

    /** Cancels loading and a running scan (tab closed). */
    fun dispose() {
        loadJob?.cancel()
        filterJob?.cancel()
        stopScan()
    }

    private fun here() = HistoryItem(
        listing?.location ?: location,
        listState.firstVisibleItemIndex,
        listState.firstVisibleItemScrollOffset,
    )

    private fun navigate(target: String, restore: HistoryItem) {
        // Opening another folder clears the name filter (desktop rule); other criteria stay.
        filter.text = ""
        selection.clear()
        collapsed = emptySet()
        location = target
        load(scrollTo = restore)
    }

    private fun load(scrollTo: HistoryItem?) {
        loadJob?.cancel()
        stopScan()
        stale = false
        val current = ++generation
        loadJob = scope.launch {
            loading = true
            try {
                val opts = options()
                val wanted = filter.spec()
                filter.error = wanted?.let { FilesApi.validateFilter(it) }
                val valid = wanted?.takeIf { filter.error == null }
                val recursive = filter.open && filter.recursive
                // The flat listing carries title, path bar and permissions; in recursive mode the
                // filter applies to the scan only.
                val result = FilesApi.list(location, opts.showHidden, if (recursive) null else valid, opts.sort)
                listing = result
                location = result.location
                error = null
                // Not suspending: applied at the next layout, even while a finger holds the list or
                // before it was ever laid out, so neither can skip the scan or keep `loading` set.
                scrollTo?.let { listState.requestScrollToItem(it.index, it.offset) }
                if (recursive && filter.error == null) startScan(result, valid, opts)
            } catch (e: CoreException) {
                listing = null
                error = e
            } finally {
                if (current == generation) {
                    loading = false
                    refreshing = false
                }
            }
        }
    }

    private fun startScan(base: Listing, spec: FilterSpec?, opts: ViewOptions) {
        scan = ScanWindow(
            scope = scope,
            root = base.location,
            filter = spec,
            local = base.isLocal,
            showHidden = opts.showHidden,
            sort = { options().sort },
            collapsed = { collapsed },
        ).also { it.start() }
    }

    private fun stopScan() {
        scan?.stop()
        scan = null
    }

    private companion object {
        const val FILTER_DEBOUNCE_MS = 350L
    }
}
