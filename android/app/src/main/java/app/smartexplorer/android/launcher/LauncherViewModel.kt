package app.smartexplorer.android.launcher

import android.app.Activity
import android.app.Application
import android.appwidget.AppWidgetProviderInfo
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.pm.LauncherApps
import android.content.pm.ShortcutInfo
import android.graphics.Rect
import android.os.UserHandle
import android.os.UserManager
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.viewModelScope
import app.smartexplorer.android.launcher.apps.AppCatalog
import app.smartexplorer.android.launcher.apps.AppEntry
import app.smartexplorer.android.launcher.apps.IconLoader
import app.smartexplorer.android.launcher.apps.ProfileInfo
import app.smartexplorer.android.launcher.badges.Badges
import app.smartexplorer.android.launcher.model.Cell
import app.smartexplorer.android.launcher.model.DropOutcome
import app.smartexplorer.android.launcher.model.DropPlanner
import app.smartexplorer.android.launcher.model.GridGeometry
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.HomeLayout
import app.smartexplorer.android.launcher.model.Kinds
import app.smartexplorer.android.launcher.model.LayoutOps
import app.smartexplorer.android.launcher.model.LayoutRepository
import app.smartexplorer.android.launcher.model.Location
import app.smartexplorer.android.launcher.model.Span
import app.smartexplorer.android.launcher.shortcuts.Shortcuts
import app.smartexplorer.android.launcher.widgets.LauncherWidgetHost
import app.smartexplorer.android.launcher.widgets.PendingWidget
import app.smartexplorer.android.launcher.widgets.REQ_WIDGET_BIND
import app.smartexplorer.android.launcher.widgets.REQ_WIDGET_CONFIGURE
import app.smartexplorer.android.launcher.widgets.Restored
import app.smartexplorer.android.launcher.widgets.ResizeLimits
import app.smartexplorer.android.launcher.widgets.Side
import app.smartexplorer.android.launcher.widgets.SizeGrids
import app.smartexplorer.android.launcher.widgets.WidgetFlow
import app.smartexplorer.android.launcher.widgets.WidgetResizing
import app.smartexplorer.android.launcher.widgets.WidgetSizes
import app.smartexplorer.android.launcher.widgets.WidgetStep
import app.smartexplorer.android.launcher.widgets.resizeLimitsOf
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** Where a long press that opened the menu happened. */
enum class MenuSource { Home, Drawer, Folder, Search }

/** The item a menu is open for; [anchor] is the item's centre in root pixels. */
data class MenuTarget(val item: HomeItem, val source: MenuSource, val anchor: Offset)

/**
 * State and actions of the home screen (DSK-M10). The arrangement itself lives in [LayoutRepository]; every
 * change of the person goes through [edit], which starts from the **displayed** layout and writes the
 * displayed cells (U35-19). No `AppNav.token`, no core: this is the `:launcher` process (U35-11, U35-16).
 */
class LauncherViewModel(application: Application, private val state: SavedStateHandle) : AndroidViewModel(application) {
    val repository: LayoutRepository = LayoutRepository.get(application)
    val catalog: AppCatalog = AppCatalog.get(application)
    val icons: IconLoader = IconLoader.get(application)
    val shortcuts: Shortcuts = Shortcuts(application)
    private val launcherApps: LauncherApps = application.getSystemService(LauncherApps::class.java)
    private val users: UserManager = application.getSystemService(UserManager::class.java)

    /** The widget host and the flow around it (U38-4, U38-7); the host listens between `onStart` and `onStop` of the activity. */
    val widgetHost = LauncherWidgetHost(application)
    val widgets = WidgetFlow(application, widgetHost)

    private val gridSize = MutableStateFlow(0 to 0)

    /**
     * The grids of both orientations for the widget sizes (U38-9). Until the home screen has measured itself this is an
     * estimate from the screen size, the same one the first arrangement uses.
     */
    var sizeGrids by mutableStateOf(estimateGrids(application, repository.layout.value.grid.iconDp))
        private set

    /** The layout as displayed in the current grid; the stored one is untouched by size changes (U35-19). */
    val display: StateFlow<HomeLayout> = combine(repository.layout, gridSize) { stored, size ->
        if (size.first <= 0 || size.second <= 0) stored else GridGeometry.placeLayout(stored, size.first, size.second)
    }.stateIn(viewModelScope, SharingStarted.Eagerly, repository.layout.value)

    /** The switch „Privaten Bereich in der App-Liste zeigen“ from `layout.json` (U38-14); the catalog follows it. */
    val showPrivateSpace: StateFlow<Boolean> = repository.layout.map { LayoutOps.showPrivateSpace(it) }
        .stateIn(viewModelScope, SharingStarted.Eagerly, LayoutOps.showPrivateSpace(repository.layout.value))

    /** Pinned shortcuts by `<package>#<id>#<user serial>`, for the labels and icons of shortcut items. */
    val shortcutInfos = MutableStateFlow<Map<String, ShortcutInfo>>(emptyMap())

    var drawerOpen by mutableStateOf(false)
        private set
    var query by mutableStateOf("")
    var openFolderId by mutableStateOf<String?>(null)
        private set
    var menu by mutableStateOf<MenuTarget?>(null)
    var editSheet by mutableStateOf(false)
        private set
    var moveTarget by mutableStateOf<HomeItem?>(null)

    /** The sheet „Widgets“ is open (U38-10). */
    var widgetPicker by mutableStateOf(false)
        private set

    /** The widget whose size is being changed (handles shown, U38-9). */
    var resizeId by mutableStateOf<String?>(null)
        private set

    /** The system dialog or configuration activity the activity has to start next (U38-7); it clears it after starting. */
    var widgetStep by mutableStateOf<WidgetStep?>(null)
        private set

    /** The widget on its way to the home screen; kept in the saved state so it survives the death of the process. */
    private var pendingWidget: PendingWidget?
        get() = PendingWidget.decode(state.get<String>(KEY_PENDING_WIDGET))
        set(value) {
            state[KEY_PENDING_WIDGET] = value?.encode()
        }

    /** Set by the menu entry „Umbenennen“: the folder popup opens with the name field active. */
    var renaming by mutableStateOf(false)
    var toast by mutableStateOf<String?>(null)
        private set

    /** Counts `goHome` calls; the pager scrolls to page 1 when it changes. */
    var homeTick by mutableIntStateOf(0)
        private set

    /** Page the pager shows; mirrored to the repository for items added from outside. */
    var currentPage: Int
        get() = repository.visiblePage
        set(value) {
            repository.visiblePage = value
        }

    /** True while a layer is open that "Zurück" closes (U35-17). */
    val hasOpenLayer: Boolean
        get() = menu != null || moveTarget != null || widgetPicker || resizeId != null || openFolderId != null || editSheet || drawerOpen || query.isNotEmpty()

    fun setGrid(columns: Int, rows: Int) {
        gridSize.value = columns to rows
    }

    /** The page area of the current orientation in dp, for the sizes widgets are told (U38-9). */
    fun setGeometry(areaWidthDp: Float, areaHeightDp: Float, iconDp: Int) {
        val grids = SizeGrids.of(areaWidthDp, areaHeightDp, iconDp, getApplication<Application>().resources.displayMetrics.density)
        if (grids != sizeGrids) sizeGrids = grids
    }

    fun showToast(text: String) {
        toast = text
    }

    fun clearToast() {
        toast = null
    }

    // ---- layers -------------------------------------------------------------------------------

    fun openDrawer() {
        drawerOpen = true
    }

    fun closeDrawer() {
        drawerOpen = false
        query = ""
    }

    fun openFolder(id: String) {
        openFolderId = id
    }

    fun closeFolder() {
        openFolderId = null
    }

    fun openEditSheet() {
        editSheet = true
    }

    fun openWidgetPicker() {
        if (editSheet) closeEditSheet()
        widgetPicker = true
    }

    fun closeWidgetPicker() {
        widgetPicker = false
    }

    fun startResize(item: HomeItem) {
        resizeId = item.id
    }

    fun endResize() {
        resizeId = null
    }

    fun closeEditSheet() {
        editSheet = false
        pruneEmptyPages()
    }

    /** The Home key (U35-17, U35-29): everything closes, the search is cleared, page 1 shows. */
    fun goHome() {
        drawerOpen = false
        query = ""
        openFolderId = null
        menu = null
        moveTarget = null
        widgetPicker = false
        resizeId = null
        if (editSheet) closeEditSheet()
        homeTick++
    }

    /** Closes the topmost layer; `false` when none is open. */
    fun closeTopLayer(): Boolean {
        when {
            menu != null -> menu = null
            moveTarget != null -> moveTarget = null
            widgetPicker -> widgetPicker = false
            resizeId != null -> resizeId = null
            openFolderId != null -> openFolderId = null
            editSheet -> closeEditSheet()
            query.isNotEmpty() -> query = ""
            drawerOpen -> drawerOpen = false
            else -> return false
        }
        return true
    }

    // ---- changing the arrangement -----------------------------------------------------------------

    private fun shown(stored: HomeLayout): HomeLayout {
        val (columns, rows) = gridSize.value
        return if (columns <= 0 || rows <= 0) stored else GridGeometry.placeLayout(stored, columns, rows)
    }

    /**
     * Applies [transform] to the displayed layout and stores the result placed again. `null` keeps the stored
     * layout as it is (nothing is written, U35-19). Pinned shortcuts follow the layout unless [syncPins] is false.
     */
    private fun edit(syncPins: Boolean = true, transform: (HomeLayout) -> HomeLayout?) {
        viewModelScope.launch {
            var before: HomeLayout? = null
            var after: HomeLayout? = null
            repository.update { stored ->
                val shownNow = shown(stored)
                val changed = transform(shownNow)
                if (changed == null) {
                    stored
                } else {
                    shown(changed).also {
                        before = shownNow
                        after = it
                    }
                }
            }
            val old = before
            val new = after
            if (syncPins && old != null && new != null) viewModelScope.launch(Dispatchers.Default) { syncPins(old, new) }
        }
    }

    private fun materialize(item: HomeItem, layout: HomeLayout): HomeItem =
        if (item.id.startsWith(TRANSIENT_PREFIX)) item.copy(id = LayoutOps.newId(layout)) else item

    fun dropOnPage(dragged: HomeItem, page: Int, cell: Cell, merge: Boolean) = edit { layout ->
        when (val outcome = DropPlanner.dropOnPage(layout, materialize(dragged, layout), page, cell, merge)) {
            is DropOutcome.Applied -> outcome.layout
            DropOutcome.NoSpace -> {
                showToast(DropPlanner.NO_SPACE_TEXT)
                null
            }
        }
    }

    fun dropOnDock(dragged: HomeItem, index: Int, merge: Boolean) = edit { layout ->
        when (val outcome = DropPlanner.dropOnDock(layout, materialize(dragged, layout), index, merge)) {
            is DropOutcome.Applied -> outcome.layout
            DropOutcome.NoSpace -> {
                showToast(DropPlanner.NO_SPACE_TEXT)
                null
            }
        }
    }

    /** „Verschieben nach…“ of the accessibility action: page, column and row count from 1 (U35-32). */
    fun moveTo(item: HomeItem, page: Int, column: Int, row: Int) = dropOnPage(item, page - 1, Cell(column - 1, row - 1), merge = false)

    fun removeItem(id: String) = edit { LayoutOps.remove(it, id) }

    fun dissolveFolder(id: String) = edit { LayoutOps.dissolveFolder(it, id) }

    fun renameFolder(id: String, name: String) = edit { LayoutOps.renameFolder(it, id, name) }

    /** „Zum Startbildschirm“ of the app list. */
    fun addToHome(entry: AppEntry) = edit { layout ->
        LayoutOps.addAnywhere(layout, materialize(appItem(entry), layout), currentPage)
    }

    fun setIconDp(iconDp: Int) = edit { it.copy(grid = it.grid.copy(iconDp = iconDp)) }

    // ---- profiles and the private space (U38-13, U38-14) ---------------------------------------------

    /** The switch of the settings sheet; stored in `layout.json` without touching any cell. */
    fun setShowPrivateSpace(on: Boolean) {
        viewModelScope.launch { repository.update { LayoutOps.withShowPrivateSpace(it, on) } }
    }

    /**
     * Pauses or resumes the work profile, locks or unlocks the private space (`requestQuietModeEnabled`, API 28). It works
     * only for the default home app in the foreground (R60); a refusal is told. The result arrives with the profile broadcasts.
     */
    fun setQuiet(profile: ProfileInfo, quiet: Boolean) {
        val accepted = try {
            users.requestQuietModeEnabled(quiet, profile.user)
        } catch (_: SecurityException) {
            false
        }
        if (!accepted) {
            showToast(
                when {
                    profile.isPrivate && quiet -> "Der private Bereich konnte nicht gesperrt werden."
                    profile.isPrivate -> "Der private Bereich konnte nicht entsperrt werden."
                    else -> "Das Arbeitsprofil konnte nicht umgeschaltet werden."
                },
            )
        }
    }

    /** A profile broadcast arrived (`ProfileReceivers`): read the apps and the paused states again. */
    fun onProfileChanged() = reloadCatalog()

    /** Fallback when a device sends no profile broadcast (A35): the state is read on every resume. */
    fun onResume() = reloadCatalog()

    // ---- widgets (U38) ----------------------------------------------------------------------------

    /** „Hinzufügen“ of the picker (U38-7): allocate an id, bind it (with the system dialog when needed), set it up, place it. */
    fun addWidget(info: AppWidgetProviderInfo, span: Span) {
        widgetPicker = false
        handle(widgets.begin(info, span, WidgetSizes.forSpan(span, sizeGrids.portrait, sizeGrids.landscape)))
    }

    /** „Erlauben“ on a cell whose widget lost its id (U38-6 step 3). */
    fun allowWidget(item: HomeItem) {
        val provider = item.provider
        val info = provider?.let { widgets.findProvider(it, item.userSerial) }
        if (info == null) {
            showToast("Das Widget ist nicht verfügbar.")
            return
        }
        handle(widgets.begin(info, item.span, WidgetSizes.forSpan(item.span, sizeGrids.portrait, sizeGrids.landscape), itemId = item.id))
    }

    /** „Einrichten“ on a cell whose configuration is open, and „Neu einrichten“ of the menu (U38-8). */
    fun setUpWidget(item: HomeItem) {
        val id = item.widgetId ?: return
        val provider = item.provider ?: return
        handle(widgets.configureStep(PendingWidget(id, provider, item.userSerial, item.span, PendingWidget.Stage.Configure, item.id)))
    }

    /** The result of a system dialog the activity started for a widget (U38-7, `LauncherActivity.onActivityResult`). */
    fun onWidgetResult(requestCode: Int, resultCode: Int, @Suppress("UNUSED_PARAMETER") data: Intent?) {
        val waiting = pendingWidget ?: return
        val ok = resultCode == Activity.RESULT_OK
        when (requestCode) {
            REQ_WIDGET_BIND -> handle(widgets.onBindResult(waiting, ok))
            REQ_WIDGET_CONFIGURE -> handle(widgets.onConfigureResult(waiting, ok))
        }
    }

    fun clearWidgetStep() {
        widgetStep = null
    }

    private fun handle(step: WidgetStep) {
        when (step) {
            is WidgetStep.AskBind -> {
                pendingWidget = step.pending
                widgetStep = step
            }
            is WidgetStep.AskConfigure -> {
                pendingWidget = step.pending
                widgetStep = step
            }
            is WidgetStep.Finished -> {
                pendingWidget = null
                finishWidget(step.pending)
            }
            is WidgetStep.Failed -> {
                pendingWidget = null
                step.message?.let { showToast(it) }
            }
        }
    }

    /** Places a new widget, or writes the new id and state into the item it belongs to (U38-6, U38-9). */
    private fun finishWidget(widget: PendingWidget) {
        val itemId = widget.itemId
        if (itemId == null) {
            edit(syncPins = false) { layout ->
                val item = HomeItem(
                    id = LayoutOps.newId(layout),
                    kind = Kinds.WIDGET,
                    span = widget.span,
                    attrs = JsonObject(
                        mapOf(
                            "provider" to JsonPrimitive(widget.provider),
                            "user" to JsonPrimitive(widget.userSerial),
                            "widgetId" to JsonPrimitive(widget.id),
                            "configured" to JsonPrimitive(true),
                        ),
                    ),
                )
                LayoutOps.addAnywhere(layout, item, currentPage)
            }
            return
        }
        val configured = if (widget.stage == PendingWidget.Stage.Configure) {
            true
        } else {
            val info = widgets.manager.getAppWidgetInfo(widget.id)
            info == null || !widgets.needsConfiguration(info)
        }
        viewModelScope.launch {
            repository.update { layout ->
                LayoutOps.updateItem(layout, itemId) { it.withAttr("widgetId", JsonPrimitive(widget.id)).withAttr("configured", JsonPrimitive(configured)) }
            }
        }
    }

    /** Checks every widget of the arrangement against the system after the activity started (U38-6). */
    private suspend fun restoreWidgets() {
        val items = LayoutOps.allItems(repository.layout.value).filter { it.isWidget }
        if (items.isEmpty()) return
        val grids = sizeGrids
        val reachable = catalog.usableProfiles
        for (item in items) {
            val sizes = WidgetSizes.forSpan(item.span, grids.portrait, grids.landscape)
            when (val restored = widgets.restore(item, sizes, item.userSerial in reachable)) {
                Restored.Keep, Restored.NeedsAllow -> Unit
                is Restored.Rebound -> repository.update { layout ->
                    LayoutOps.updateItem(layout, item.id) {
                        it.withAttr("widgetId", JsonPrimitive(restored.widgetId)).withAttr("configured", JsonPrimitive(restored.configured))
                    }
                }
                Restored.Gone -> repository.update { LayoutOps.remove(it, item.id) }
            }
        }
    }

    /** Frees the id of every widget that leaves the arrangement, whichever way it leaves (U38-6). */
    private suspend fun releaseRemovedWidgets() {
        var known = LayoutOps.widgetIds(repository.layout.value)
        repository.layout.collect { layout ->
            val now = LayoutOps.widgetIds(layout)
            (known - now).forEach { widgets.release(it) }
            known = now
        }
    }

    /** The limits of [item]'s size in the displayed grid, or `null` while the grid is unknown or the widget unbound. */
    private fun limitsOf(item: HomeItem): ResizeLimits? {
        val info = item.widgetId?.let { widgets.manager.getAppWidgetInfo(it) } ?: return null
        val (columns, rows) = gridSize.value
        if (columns <= 0 || rows <= 0) return null
        val grids = sizeGrids
        return resizeLimitsOf(info, item.span, grids.current.cellWidthDp * grids.density, grids.current.cellHeightDp * grids.density, columns, rows)
    }

    /** True when the menu offers „Größe ändern“ for [item]: the provider lets at least one direction change. */
    fun canResize(item: HomeItem): Boolean = limitsOf(item)?.any == true

    /** Stores the new area of a widget after its handles were pulled (U38-9). */
    fun setWidgetArea(id: String, cell: Cell, span: Span) = edit(syncPins = false) { LayoutOps.setArea(it, id, cell, span) }

    /**
     * One cell wider, narrower, taller or lower (the accessibility alternative to the handles): the side that
     * allows it is moved. Returns whether anything changed.
     */
    fun stepWidget(item: HomeItem, horizontal: Boolean, grow: Boolean): Boolean {
        val limits = limitsOf(item) ?: return false
        val (columns, rows) = gridSize.value
        val found = LayoutOps.find(display.value, item.id) ?: return false
        val page = (found.location as? Location.Page)?.index ?: return false
        val taken = GridGeometry.occupied(display.value.pages[page].filter { it.id != item.id })
        val attempts = when {
            horizontal && grow -> listOf(Side.Right to 1, Side.Left to -1)
            horizontal -> listOf(Side.Right to -1, Side.Left to 1)
            grow -> listOf(Side.Bottom to 1, Side.Top to -1)
            else -> listOf(Side.Bottom to -1, Side.Top to 1)
        }
        for ((side, delta) in attempts) {
            val area = WidgetResizing.pull(item.cell, item.span, side, delta, limits, taken, columns, rows) ?: continue
            setWidgetArea(item.id, area.first, area.second)
            return true
        }
        showToast("Die Größe lässt sich nicht weiter ändern.")
        return false
    }

    /** Drops empty pages except the first when the edit mode ends (U35-22). */
    fun pruneEmptyPages() = edit(syncPins = false) { layout ->
        val pruned = LayoutOps.pruneEmptyPages(layout)
        if (pruned.pages.size == layout.pages.size) null else pruned
    }

    private suspend fun syncPins(before: HomeLayout, after: HomeLayout) {
        val old = pinsOf(before)
        val new = pinsOf(after)
        for (key in old.keys + new.keys) {
            val ids = new[key].orEmpty()
            if (old[key].orEmpty() != ids) {
                val user = userFor(key.second) ?: continue
                shortcuts.pin(key.first, ids.toList(), user)
            }
        }
        refreshShortcuts()
    }

    private fun pinsOf(layout: HomeLayout): Map<Pair<String, Long>, Set<String>> =
        LayoutOps.allItems(layout).filter { it.isShortcut && it.packageName != null }
            .groupBy { (it.packageName ?: "") to it.userSerial }
            .mapValues { entry -> entry.value.mapNotNull { it.attr("shortcutId") }.toSet() }

    private fun userFor(serial: Long): UserHandle? = users.getUserForSerialNumber(serial)

    // ---- apps -------------------------------------------------------------------------------------

    fun entryFor(item: HomeItem): AppEntry? = item.component?.let { catalog.find(it, item.userSerial) }

    fun appItem(entry: AppEntry): HomeItem = HomeItem(
        id = TRANSIENT_PREFIX + entry.key,
        kind = Kinds.APP,
        attrs = JsonObject(mapOf("component" to JsonPrimitive(entry.componentString), "user" to JsonPrimitive(entry.userSerial))),
    )

    /** Starts what a tap on [item] means: an app, a pinned shortcut, or opens a folder. [bounds] animates the start. */
    fun launch(item: HomeItem, bounds: Rect? = null) {
        when {
            item.isApp -> launchApp(item, bounds)
            item.isShortcut -> launchShortcut(item)
            item.isFolder -> openFolder(item.id)
        }
    }

    private fun launchApp(item: HomeItem, bounds: Rect?) {
        val entry = entryFor(item)
        if (entry == null) {
            showToast("${item.packageName ?: "Die App"} ist nicht verfügbar.")
            return
        }
        launchEntry(entry, bounds)
    }

    fun launchEntry(entry: AppEntry, bounds: Rect? = null) {
        if (!entry.available) {
            showToast("${entry.label} ist nicht verfügbar.")
            return
        }
        try {
            launcherApps.startMainActivity(entry.component, entry.user, bounds, null)
            if (drawerOpen) closeDrawer()
        } catch (_: SecurityException) {
            showToast("${entry.label} ist nicht verfügbar.")
        } catch (_: ActivityNotFoundException) {
            showToast("${entry.label} ist nicht verfügbar.")
        }
    }

    private fun launchShortcut(item: HomeItem) {
        val pkg = item.packageName
        val id = item.attr("shortcutId")
        val user = userFor(item.userSerial)
        if (pkg == null || id == null || user == null || !shortcuts.startById(pkg, id, user)) {
            showToast("${shortcutInfos.value[shortcutKey(item)]?.shortLabel ?: "Die Verknüpfung"} ist nicht verfügbar.")
        } else if (drawerOpen) {
            closeDrawer()
        }
    }

    fun startAppInfo(item: HomeItem) {
        val entry = entryFor(item) ?: return
        try {
            launcherApps.startAppDetailsActivity(entry.component, entry.user, null, null)
        } catch (_: ActivityNotFoundException) {
            showToast("App-Info ist nicht verfügbar.")
        }
    }

    // ---- life cycle and system callbacks -------------------------------------------------------

    /** Reads the apps and checks the arrangement against them (called from `onStart`). */
    fun onStart() {
        viewModelScope.launch(Dispatchers.Default) {
            catalog.reload()
            refreshShortcuts()
            pruneDeadItems()
            restoreWidgets()
        }
    }

    /** Writes whatever is queued (called from `onStop`). */
    fun onStop() = repository.flush()

    fun shortcutKey(item: HomeItem): String = "${item.packageName}#${item.attr("shortcutId")}#${item.userSerial}"

    /** Reads the pinned shortcuts of every package that has one on the home screen; a missing one is removed (U35-26). */
    private suspend fun refreshShortcuts() {
        if (!shortcuts.hasHostPermission()) return
        val layout = repository.layout.value
        val wanted = LayoutOps.allItems(layout).filter { it.isShortcut && it.packageName != null }
        val infos = HashMap<String, ShortcutInfo>()
        val missing = HashSet<String>()
        wanted.groupBy { (it.packageName ?: "") to it.userSerial }.forEach { (key, items) ->
            val user = userFor(key.second) ?: return@forEach
            val pinned = shortcuts.pinned(key.first, user).associateBy { it.id }
            for (item in items) {
                val info = pinned[item.attr("shortcutId")]
                if (info == null) missing += item.id else infos[shortcutKey(item)] = info
            }
        }
        shortcutInfos.value = infos
        if (missing.isNotEmpty()) edit(syncPins = false) { LayoutOps.removeWhere(it) { item -> item.id in missing } }
    }

    /**
     * Removes app items whose activity no longer exists, for removals that happened while the launcher process
     * was not running (U35-29). Only profiles that are unlocked, not paused and list at least one app are
     * asked, so a profile that is merely not reachable never loses its items.
     */
    private fun pruneDeadItems() {
        val reliable = catalog.usableProfiles intersect catalog.apps.value.map { it.userSerial }.toSet()
        val known = catalog.apps.value.map { it.key }.toSet()
        if (reliable.isEmpty()) return
        edit(syncPins = false) { layout ->
            val dead = LayoutOps.allItems(layout).filter {
                it.isApp && it.userSerial in reliable && it.component != null && "${it.component}#${it.userSerial}" !in known
            }.map { it.id }.toSet()
            if (dead.isEmpty()) null else LayoutOps.removeWhere(layout) { it.id in dead }
        }
    }

    /** Package changes (U35-29); registered in `onStart`, unregistered in `onStop`. */
    val callback: LauncherApps.Callback = object : LauncherApps.Callback() {
        override fun onPackageRemoved(packageName: String, user: UserHandle) {
            icons.invalidate(packageName)
            val serial = users.getSerialNumberForUser(user)
            reloadCatalog()
            edit(syncPins = false) { layout ->
                val removed = LayoutOps.removePackage(layout, packageName, serial)
                if (removed == layout) null else removed
            }
        }

        override fun onPackageAdded(packageName: String, user: UserHandle) {
            // A new app appears in the app list, never on the home screen by itself (P24).
            reloadCatalog()
        }

        override fun onPackageChanged(packageName: String, user: UserHandle) {
            icons.invalidate(packageName)
            viewModelScope.launch(Dispatchers.Default) {
                catalog.reload()
                pruneDeadItems()
            }
        }

        override fun onPackagesAvailable(packageNames: Array<out String>, user: UserHandle, replacing: Boolean) = reloadCatalog()

        override fun onPackagesUnavailable(packageNames: Array<out String>, user: UserHandle, replacing: Boolean) = reloadCatalog()

        override fun onPackagesSuspended(packageNames: Array<out String>, user: UserHandle) = reloadCatalog()

        override fun onPackagesUnsuspended(packageNames: Array<out String>, user: UserHandle) = reloadCatalog()
    }

    private fun reloadCatalog() {
        viewModelScope.launch(Dispatchers.Default) { catalog.reload() }
    }

    init {
        catalog.showPrivateSpace = showPrivateSpace.value
        viewModelScope.launch(Dispatchers.Default) { releaseRemovedWidgets() }
        viewModelScope.launch(Dispatchers.Default) {
            showPrivateSpace.collect {
                catalog.showPrivateSpace = it
            }
        }
        // A locked private space shows no dot (U38-14).
        viewModelScope.launch(Dispatchers.Default) {
            catalog.profiles.collect { profiles -> Badges.setHiddenUsers(profiles.filter { it.isHidden }.map { it.userSerial }.toSet()) }
        }
    }

    companion object {
        /** Ids of items that are not in the layout yet (dragged out of the app list); a real id is given on the drop. */
        const val TRANSIENT_PREFIX = "drag:"

        private const val KEY_PENDING_WIDGET = "pendingWidget"

        /** Before the home screen has measured itself: the page area of the screen without insets, dock and page dots. */
        private fun estimateGrids(application: Application, iconDp: Int): SizeGrids {
            val metrics = application.resources.displayMetrics
            val widthDp = metrics.widthPixels / metrics.density
            val heightDp = metrics.heightPixels / metrics.density - GridGeometry.dockHeightDp(iconDp) - GridGeometry.INDICATOR_DP - 96f
            return SizeGrids.of(widthDp, heightDp, iconDp, metrics.density)
        }
    }
}
