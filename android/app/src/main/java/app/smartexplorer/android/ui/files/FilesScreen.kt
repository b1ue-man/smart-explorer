package app.smartexplorer.android.ui.files

import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.PermanentDrawerSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.adaptive.ExperimentalMaterial3AdaptiveApi
import androidx.compose.material3.adaptive.currentWindowAdaptiveInfo
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.window.core.layout.WindowSizeClass
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.system.Opener
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.MainTab
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.AllFilesAccessBanner
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.common.rememberAllFilesAccess
import app.smartexplorer.android.ui.more.MoreRequests
import app.smartexplorer.android.ui.transfers.TransfersBar
import app.smartexplorer.android.ui.trash.TrashScreen
import kotlinx.coroutines.launch

/** Backends that exist only inside the app and never become favorites (api.md §2). */
private val INTERNAL_BACKENDS = setOf("zip", "trash")

/**
 * Tab "Dateien" (spec B F3–F11, C): places sidebar (drawer below 840 dp, permanent above), one
 * or two panes with title bar, path bar, filter row and list, bottom bars for transfers, paste and
 * changed remote copies, (+) for new items. Handles `NavRequest.OpenLocation`/`ShowTransfers`.
 */
@OptIn(ExperimentalMaterial3AdaptiveApi::class)
@Composable
fun FilesScreen() {
    val vm: FilesViewModel = viewModel()
    val context = LocalContext.current
    val currentContext by rememberUpdatedState(context)
    val wide = currentWindowAdaptiveInfo().windowSizeClass
        .isWidthAtLeastBreakpoint(WindowSizeClass.WIDTH_DP_EXPANDED_LOWER_BOUND)
    val tasks by Core.tasks.collectAsStateWithLifecycle()
    val pending by AppNav.pending.collectAsStateWithLifecycle()
    val access = rememberAllFilesAccess()
    val seenAccess = remember { booleanArrayOf(access) }

    LaunchedEffect(wide) { vm.setPanes(if (wide) 2 else 1) }
    LaunchedEffect(pending) {
        when (val request = pending) {
            is NavRequest.OpenLocation -> {
                vm.openLocation(request.location)
                AppNav.consume(request)
            }
            NavRequest.ShowTransfers -> {
                vm.sheet = FilesSheet.Transfers
                AppNav.consume(request)
            }
            else -> Unit
        }
    }
    // Opening or sharing needs a visible activity (background starts are blocked): effects stay
    // buffered in the channel while the screen is stopped and run once it is started again.
    val lifecycleOwner = LocalLifecycleOwner.current
    LaunchedEffect(vm, lifecycleOwner) {
        lifecycleOwner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            vm.effects.collect { perform(currentContext, it) }
        }
    }
    LaunchedEffect(access) {
        // Access granted or revoked in the system settings: places and lists change.
        if (access != seenAccess[0]) {
            seenAccess[0] = access
            vm.reloadRoots()
            vm.refreshAfterChange()
        }
    }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { vm.onResume() }

    Box(Modifier.fillMaxSize()) {
        when (vm.page) {
            FilesPage.Trash -> TrashScreen(onBack = {
                vm.page = FilesPage.Browser
                vm.refreshAfterChange()
            })
            FilesPage.FolderSearch -> FolderSearchPage(
                vm.search,
                onOpen = { vm.openLocation(it) },
                onBack = { vm.page = FilesPage.Browser },
            )
            FilesPage.Browser -> FilesBrowser(vm, wide, tasks)
        }
    }
    FilesOverlays(vm, tasks)
}

@Composable
private fun FilesBrowser(vm: FilesViewModel, wide: Boolean, tasks: List<TaskInfo>) {
    val drawerState = rememberDrawerState(DrawerValue.Closed)
    val scope = rememberCoroutineScope()
    val closeDrawer: () -> Unit = { scope.launch { drawerState.close() } }
    val places = PlacesCallbacks(
        onOpen = { root ->
            vm.openRoot(root)
            closeDrawer()
        },
        onFolderSearch = {
            vm.page = FilesPage.FolderSearch
            closeDrawer()
        },
        onRemoveFavorite = { vm.toggleFavorite(it.location) },
        onManageConnections = {
            // Connections and Google Drive are managed on "Mehr" (K3).
            MoreRequests.open(MoreRequests.Request.AddConnection)
            closeDrawer()
        },
        onRetry = {
            vm.reloadRoots()
            vm.start()
        },
    )
    val current = vm.activeTab?.listing?.location
    if (wide) {
        Row(Modifier.fillMaxSize()) {
            PermanentDrawerSheet(Modifier.width(280.dp)) { PlacesPanel(vm.roots, vm.rootsError, current, places) }
            BrowserBody(vm, wide = true, tasks = tasks, onMenu = {}, backEnabled = true, modifier = Modifier.weight(1f))
        }
    } else {
        ModalNavigationDrawer(
            drawerState = drawerState,
            gesturesEnabled = drawerState.isOpen,
            drawerContent = { ModalDrawerSheet(drawerState) { PlacesPanel(vm.roots, vm.rootsError, current, places) } },
        ) {
            // While the drawer is open, back closes it (ModalDrawerSheet) instead of changing folders.
            BrowserBody(
                vm,
                wide = false,
                tasks = tasks,
                onMenu = { scope.launch { drawerState.open() } },
                backEnabled = !drawerState.isOpen,
            )
        }
    }
}

@Composable
private fun BrowserBody(
    vm: FilesViewModel,
    wide: Boolean,
    tasks: List<TaskInfo>,
    onMenu: () -> Unit,
    backEnabled: Boolean,
    modifier: Modifier = Modifier,
) {
    val compact by AppPrefs.compact.collectAsStateWithLifecycle()
    val thumbnails by AppPrefs.thumbnails.collectAsStateWithLifecycle()
    val active = vm.activeTab
    Scaffold(
        modifier = modifier.fillMaxSize().imePadding().navigationBarsPadding(),
        floatingActionButton = {
            if (active != null && active.writableLocation != null && active.selection.isEmpty()) {
                NewItemFab { folder -> vm.requestCreate(active, folder) }
            }
        },
        bottomBar = { BottomBars(vm, tasks) },
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
    ) { padding ->
        val shown = if (wide) vm.paneTabs.take(2) else listOfNotNull(vm.paneTabs.getOrNull(vm.activePane))
        if (shown.isEmpty()) {
            StartState(vm, Modifier.padding(padding))
        } else {
            Row(Modifier.fillMaxSize().padding(padding)) {
                shown.forEachIndexed { index, id ->
                    val tab = vm.tab(id)
                    if (tab != null) {
                        val pane = if (wide) index else vm.activePane
                        key(id) {
                            PaneView(
                                vm = vm,
                                pane = pane,
                                tab = tab,
                                wide = wide,
                                onMenu = onMenu,
                                backEnabled = backEnabled,
                                tasks = tasks,
                                compact = compact,
                                thumbnails = thumbnails,
                                modifier = Modifier.weight(1f).fillMaxHeight(),
                            )
                        }
                    }
                }
            }
        }
    }
}

/** Before the first tab exists: start indicator or the reason it failed. */
@Composable
private fun StartState(vm: FilesViewModel, modifier: Modifier) {
    Column(modifier.fillMaxSize().statusBarsPadding()) {
        val error = vm.rootsError
        if (error == null) {
            LoadingBar(loading = true)
        } else {
            ErrorCard(error, modifier = Modifier.padding(16.dp), title = "Speicherorte nicht verfügbar", actionLabel = "Erneut", onAction = { vm.start() })
        }
    }
}

@Composable
private fun BottomBars(vm: FilesViewModel, tasks: List<TaskInfo>) {
    Column {
        AllFilesAccessBanner()
        vm.edits.forEach { edit ->
            EditCard(edit, onUpload = { vm.uploadEdit(edit, mode = "overwrite") }, onDiscard = { vm.discardEdit(edit) })
        }
        TransfersBar(tasks, onShow = { vm.sheet = FilesSheet.Transfers })
        vm.clip?.let { clip ->
            val target = vm.activeTab
            PasteBar(
                clip = clip,
                canPaste = target?.writableLocation != null,
                onPaste = { target?.let { vm.paste(it) } },
                onClear = { vm.clip = null },
            )
        }
    }
}

/** One pane: title bar (or selection bar), path bar, filter row, list, footer. */
@Composable
private fun PaneView(
    vm: FilesViewModel,
    pane: Int,
    tab: BrowserTab,
    wide: Boolean,
    onMenu: () -> Unit,
    backEnabled: Boolean,
    tasks: List<TaskInfo>,
    compact: Boolean,
    thumbnails: Boolean,
    modifier: Modifier,
) {
    val scope = rememberCoroutineScope()
    val isActive = pane == vm.activePane
    val listing = tab.listing
    val selectionMode = tab.selection.isNotEmpty()
    val local = tab.scan?.local ?: (listing?.isLocal == true)
    val readOnly = listing?.readOnly ?: true
    val internal = listing != null && listing.backend in INTERNAL_BACKENDS

    // The tab outlives this activity: a hidden tab must not keep the old layout tree alive.
    DisposableEffect(tab) { onDispose { tab.detachList() } }
    BackHandler(enabled = backEnabled && isActive && (selectionMode || tab.canGoBack)) {
        if (selectionMode) tab.selection.clear() else tab.goBack()
    }

    val frame = if (wide) {
        Modifier.border(2.dp, if (isActive) MaterialTheme.colorScheme.primary else Color.Transparent)
    } else {
        Modifier
    }
    Column(
        modifier
            .then(frame)
            // Any touch inside a pane makes it the active one (paste target, tab switcher); the
            // event is only observed, never consumed. Restarts when the pane index changes.
            .pointerInput(pane) {
                awaitPointerEventScope {
                    while (true) {
                        awaitPointerEvent(PointerEventPass.Initial)
                        if (vm.activePane != pane) vm.activePane = pane
                    }
                }
            },
    ) {
        if (selectionMode) {
            val entries = tab.selection.values.toList()
            SelectionTopBar(
                count = entries.size,
                canModify = !readOnly,
                canCut = !readOnly && local,
                canRename = !readOnly && entries.size == 1,
                canExtract = entries.size == 1 && isZip(entries.first()),
                canFavorite = !internal && entries.all { it.isDir },
                onClear = { tab.selection.clear() },
                onSelectAll = { vm.selectAll(tab) },
                onInvert = { invertSelection(tab) },
                onAction = { vm.onEntryAction(tab, it, entries, local) },
            )
        } else {
            FilesTopBar(
                title = titleOf(tab),
                location = listing?.location,
                showMenuButton = !wide,
                onMenu = onMenu,
                filterOpen = tab.filter.open,
                onToggleFilter = { if (tab.filter.open) tab.closeFilter() else tab.filter.open = true },
                tabCount = vm.tabs.size,
                onTabs = { vm.sheet = FilesSheet.Tabs },
                canForward = tab.canGoForward,
                canFavorite = listing != null && !internal,
                onAction = { vm.onFolderAction(tab, it) },
            )
        }
        CrumbBar(listing?.crumbs.orEmpty(), onOpen = { tab.open(it) })
        if (tab.filter.open) FilterBar(tab, onOpenSheet = { vm.sheet = FilesSheet.Filter(tab.id) })
        LoadingBar(tab.loading && !tab.refreshing)
        val callbacks = remember(tab) {
            ListCallbacks(
                onOpen = { entry, isLocal -> vm.openEntry(tab, entry, isLocal) },
                onAction = { action, entry, isLocal -> vm.onEntryAction(tab, action, listOf(entry), isLocal) },
                onCreateFolder = { vm.requestCreate(tab, folder = true) },
                onResetFilter = { tab.closeFilter() },
                onPlaceSettings = { MoreRequests.open(MoreRequests.Request.Connections) },
            )
        }
        FileListArea(tab, compact, thumbnails, callbacks, Modifier.weight(1f))
        ListFooter(
            tab,
            tasks,
            onStop = {
                scope.launch {
                    try {
                        tab.scan?.cancelScan()
                    } catch (e: CoreException) {
                        Snackbars.show("Scan ließ sich nicht stoppen: ${e.message ?: e.kind}")
                    }
                }
            },
            onIssues = { vm.showScanIssues(it) },
        )
    }
}

/** Title = place; ZIP contents are marked read-only (spec F10). */
private fun titleOf(tab: BrowserTab): String {
    val listing = tab.listing ?: return tab.location
    val title = listing.title.ifBlank { listing.location }
    return if (listing.backend == "zip" && !title.contains("nur lesen")) "$title (nur lesen)" else title
}

private fun FilesViewModel.onEntryAction(tab: BrowserTab, action: EntryAction, entries: List<Entry>, local: Boolean) {
    when (action) {
        EntryAction.OpenWith -> entries.singleOrNull()?.let { openFile(it, local, chooser = true) }
        EntryAction.Share -> share(entries)
        EntryAction.Copy -> copyToClip(tab, entries, move = false)
        EntryAction.Cut -> copyToClip(tab, entries, move = true)
        EntryAction.CopyTo -> requestTransferTo(tab, entries, move = false)
        EntryAction.MoveTo -> requestTransferTo(tab, entries, move = true)
        EntryAction.Rename -> entries.singleOrNull()?.let { requestRename(tab, it) }
        EntryAction.Delete -> requestDelete(tab, entries)
        EntryAction.Properties -> showProperties(entries, tab.scan)
        EntryAction.Extract -> entries.singleOrNull()?.let { requestExtract(tab, it) }
        EntryAction.CopyPath -> copyPaths(entries.map { it.location })
        EntryAction.Favorite -> addFavorites(tab, entries)
    }
}

private fun FilesViewModel.onFolderAction(tab: BrowserTab, action: FolderAction) {
    val location = tab.listing?.location
    when (action) {
        FolderAction.ViewOptions -> sheet = FilesSheet.ViewOptions
        FolderAction.FolderSearch -> page = FilesPage.FolderSearch
        FolderAction.NewTab -> newTab(null)
        FolderAction.Forward -> tab.goForward()
        FolderAction.Mirror -> location?.let { sheet = FilesSheet.Mirror(it) }
        FolderAction.Analyze -> location?.let { MoreRequests.open(MoreRequests.Request.Analyze(it)) }
        FolderAction.Favorite -> location?.let { toggleFavorite(it) }
        FolderAction.CopyPath -> location?.let { copyPaths(listOf(it)) }
        FolderAction.Properties -> showFolderProperties(tab)
    }
}

/** Runs an action that needs the current activity (open, share, clipboard). */
private fun perform(context: Context, effect: FilesEffect) {
    when (effect) {
        is FilesEffect.Open -> when (Opener.open(context, effect.localPath, effect.mime, effect.chooser)) {
            Opener.Outcome.Started -> Unit
            Opener.Outcome.NoApp -> Snackbars.show("Keine App für diesen Dateityp")
            Opener.Outcome.NotShareable -> Snackbars.show("Diese Datei kann nicht an andere Apps übergeben werden.")
        }
        is FilesEffect.Share -> when (Opener.share(context, effect.paths)) {
            Opener.Outcome.Started -> if (effect.skippedFolders > 0) Snackbars.show("Ordner ausgelassen: ${effect.skippedFolders}")
            Opener.Outcome.NoApp -> Snackbars.show("Keine App zum Teilen gefunden")
            Opener.Outcome.NotShareable -> Snackbars.show("Diese Dateien können nicht geteilt werden.")
        }
        is FilesEffect.CopyText -> {
            if (Opener.copyText(context, effect.label, effect.text)) {
                Snackbars.show("${effect.label} kopiert")
            } else {
                Snackbars.show("Zwischenablage nicht verfügbar")
            }
        }
    }
}
