package app.smartexplorer.android.ui.files

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.smartexplorer.android.api.EditInfo
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.Roots
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Root
import app.smartexplorer.android.core.SortSpec
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.trash.TRASH_RETENTION_DAYS
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch

/**
 * State of the Files tab, scoped to the activity so places, tabs, history, filters and the
 * clipboard survive tab switches and rotation. Operations live in FileActions.kt.
 */
internal class FilesViewModel : ViewModel() {
    val tabs = mutableStateListOf<BrowserTab>()

    /** Tab id shown per pane; the second pane exists from the first wide layout on. */
    val paneTabs = mutableStateListOf<Long>()
    var activePane by mutableIntStateOf(0)
    var paneCount by mutableIntStateOf(1)
        private set
    var page by mutableStateOf(FilesPage.Browser)
    var roots by mutableStateOf<Roots?>(null)
        private set
    var rootsError by mutableStateOf<String?>(null)
        private set
    var clip by mutableStateOf<Clip?>(null)
    var edits by mutableStateOf<List<EditInfo>>(emptyList())
        private set
    var sortKey by mutableStateOf("name")
        private set
    var sortDesc by mutableStateOf(false)
        private set
    var dialog by mutableStateOf<FilesDialog?>(null)
    var sheet by mutableStateOf<FilesSheet?>(null)
    var picker by mutableStateOf<PickerRequest?>(null)
    var openProgress by mutableStateOf<OpenProgress?>(null)
    val search = FolderSearchState(viewModelScope)

    private val effectChannel = Channel<FilesEffect>(Channel.BUFFERED)
    val effects: Flow<FilesEffect> = effectChannel.receiveAsFlow()

    private var nextTabId = 1L

    /** Place requested (e.g. from a notification) before the first tab existed. */
    private var startLocation: String? = null
    private var appliedHidden = AppPrefs.showHidden.value
    private var appliedDirsFirst = AppPrefs.dirsFirst.value

    /** Tab in the active pane. */
    val activeTab: BrowserTab?
        get() = paneTab(activePane)

    init {
        viewModelScope.launch {
            Core.events.collect { event ->
                when (event) {
                    CoreEvent.Volumes -> reloadRoots()
                    CoreEvent.Edits -> reloadEdits()
                    else -> Unit
                }
            }
        }
        viewModelScope.launch {
            combine(AppPrefs.showHidden, AppPrefs.dirsFirst) { hidden, dirsFirst -> hidden to dirsFirst }
                .collect { (hidden, dirsFirst) ->
                    if (hidden == appliedHidden && dirsFirst == appliedDirsFirst) return@collect
                    val hiddenChanged = hidden != appliedHidden
                    appliedHidden = hidden
                    appliedDirsFirst = dirsFirst
                    optionsChanged(hiddenChanged)
                }
        }
        viewModelScope.launch {
            snapshotFlow { activeTab?.listing?.location }.collect { FilesLocation.set(it) }
        }
        start()
    }

    /** First load: places, then a tab on the primary storage volume. Also the [Erneut] of a failed start. */
    fun start() {
        if (tabs.isNotEmpty()) return
        viewModelScope.launch {
            val loaded = loadRoots() ?: return@launch
            val home = startLocation ?: loaded.storage.firstOrNull()?.location
            startLocation = null
            if (home == null) {
                // A concurrent start (roots reload) may have opened a tab meanwhile.
                if (tabs.isEmpty()) rootsError = "Kein Speicherort gefunden. Ist der Speicher eingebunden?"
                return@launch
            }
            if (tabs.isNotEmpty()) return@launch
            val tab = createTab(home)
            paneTabs.clear()
            paneTabs.add(tab.id)
            if (paneCount > 1) ensureSecondPane()
            tab.reload()
            reloadEdits()
            purgeOldTrash()
        }
    }

    /** App start: trash entries older than 30 days go (spec F11; the worker does the same). */
    private suspend fun purgeOldTrash() {
        try {
            FilesApi.trashPurge(TRASH_RETENTION_DAYS)
        } catch (e: CoreException) {
            Snackbars.show("Alte Papierkorb-Einträge ließen sich nicht entfernen: ${e.message ?: e.kind}")
        }
    }

    fun options(): ViewOptions =
        ViewOptions(AppPrefs.showHidden.value, SortSpec(key = sortKey, desc = sortDesc, dirsFirst = AppPrefs.dirsFirst.value))

    fun tab(id: Long): BrowserTab? = tabs.firstOrNull { it.id == id }

    fun paneTab(pane: Int): BrowserTab? = paneTabs.getOrNull(pane)?.let { tab(it) }

    /** Tabs visible right now (one pane below 840 dp, two above). */
    fun displayedTabs(): List<BrowserTab> =
        if (paneCount > 1) paneTabs.take(paneCount).mapNotNull { tab(it) } else listOfNotNull(activeTab)

    /** One pane below 840 dp, two side by side above (spec F3). */
    fun setPanes(count: Int) {
        paneCount = count.coerceIn(1, 2)
        if (paneCount > 1) ensureSecondPane()
        activePane = activePane.coerceIn(0, (paneTabs.size - 1).coerceAtLeast(0))
    }

    /** Location of the pane next to [tab] in the two-pane layout ("Kopieren nach…" suggestion). */
    fun otherPaneLocation(tab: BrowserTab): String? {
        if (paneCount < 2) return null
        val other = paneTabs.take(2).firstOrNull { it != tab.id } ?: return null
        return tab(other)?.listing?.location
    }

    fun newTab(location: String?) {
        val start = location ?: activeTab?.listing?.location ?: activeTab?.location ?: return
        val tab = createTab(start)
        showInActivePane(tab.id)
        page = FilesPage.Browser
        tab.reload()
    }

    /** Shows [id] in the active pane; a tab already shown in the other pane swaps places. */
    fun selectTab(id: Long) {
        showInActivePane(id)
        tab(id)?.let { if (it.stale || (it.listing == null && it.error == null && !it.loading)) it.reload() }
    }

    fun closeTab(id: Long) {
        if (tabs.size <= paneCount) return
        val tab = tab(id) ?: return
        tab.dispose()
        tabs.remove(tab)
        val pane = paneTabs.indexOf(id)
        if (pane < 0) return
        val replacement = tabs.firstOrNull { it.id !in paneTabs }
        if (replacement != null) {
            paneTabs[pane] = replacement.id
            if (replacement.stale || replacement.listing == null) replacement.reload()
        } else {
            paneTabs.removeAt(pane)
        }
        activePane = activePane.coerceIn(0, (paneTabs.size - 1).coerceAtLeast(0))
    }

    fun openLocation(location: String) {
        page = FilesPage.Browser
        val tab = activeTab
        if (tab == null) {
            startLocation = location
            start()
        } else {
            tab.open(location)
        }
    }

    fun openRoot(root: Root) {
        if (root.kind == "trash") {
            page = FilesPage.Trash
            return
        }
        openLocation(root.location)
    }

    fun setSort(key: String, desc: Boolean) {
        if (key == sortKey && desc == sortDesc) return
        sortKey = key
        sortDesc = desc
        optionsChanged(hiddenChanged = false)
    }

    /** Also finishes a failed start once places are readable again (e.g. storage mounted). */
    fun reloadRoots() {
        viewModelScope.launch {
            if (loadRoots() != null && tabs.isEmpty()) start()
        }
    }

    fun reloadEdits() {
        viewModelScope.launch {
            try {
                edits = FilesApi.edits().filter { it.modified }
            } catch (e: CoreException) {
                Snackbars.show("Geöffnete Remote-Dateien nicht prüfbar: ${e.message ?: e.kind}")
            }
        }
    }

    /** UI came back (other app closed, tab switched back): pick up external changes. */
    fun onResume() {
        reloadEdits()
        displayedTabs().filter { it.listing?.isLocal == true && !it.loading && it.scan == null }.forEach { it.reload() }
    }

    /** Reloads what a finished operation may have changed. */
    fun refreshAfterChange() {
        val shown = displayedTabs()
        shown.forEach { if (!it.loading) it.reload() }
        tabs.filter { it !in shown }.forEach { it.stale = true }
    }

    internal fun emit(effect: FilesEffect) {
        effectChannel.trySend(effect)
    }

    override fun onCleared() {
        // viewModelScope is already cancelled here; ScanWindow sends its task.cancel on its own scope.
        tabs.forEach { it.dispose() }
        super.onCleared()
    }

    private suspend fun loadRoots(): Roots? = try {
        FilesApi.roots().also {
            roots = it
            rootsError = null
        }
    } catch (e: CoreException) {
        rootsError = e.message ?: e.kind
        null
    }

    private fun createTab(location: String): BrowserTab =
        BrowserTab(nextTabId++, location, viewModelScope, ::options).also { tabs.add(it) }

    private fun ensureSecondPane() {
        if (paneTabs.size >= 2 || paneTabs.isEmpty()) return
        val free = tabs.firstOrNull { it.id !in paneTabs }
        val tab = free ?: createTab(paneTab(0)?.listing?.location ?: paneTab(0)?.location ?: return)
        paneTabs.add(tab.id)
        if (tab.listing == null || tab.stale) tab.reload()
    }

    private fun showInActivePane(id: Long) {
        if (paneTabs.isEmpty()) {
            paneTabs.add(id)
            activePane = 0
            return
        }
        val pane = activePane.coerceIn(0, paneTabs.size - 1)
        val other = paneTabs.indexOf(id)
        if (other >= 0 && other != pane) paneTabs[other] = paneTabs[pane]
        paneTabs[pane] = id
    }

    private fun optionsChanged(hiddenChanged: Boolean) {
        val shown = displayedTabs()
        tabs.forEach { tab ->
            if (tab in shown) tab.optionsChanged(hiddenChanged) else tab.stale = true
        }
    }
}
