package app.smartexplorer.android.launcher

import android.app.Application
import android.content.ActivityNotFoundException
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
import androidx.lifecycle.viewModelScope
import app.smartexplorer.android.launcher.apps.AppCatalog
import app.smartexplorer.android.launcher.apps.AppEntry
import app.smartexplorer.android.launcher.apps.IconLoader
import app.smartexplorer.android.launcher.model.Cell
import app.smartexplorer.android.launcher.model.DropOutcome
import app.smartexplorer.android.launcher.model.DropPlanner
import app.smartexplorer.android.launcher.model.GridGeometry
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.HomeLayout
import app.smartexplorer.android.launcher.model.Kinds
import app.smartexplorer.android.launcher.model.LayoutOps
import app.smartexplorer.android.launcher.model.LayoutRepository
import app.smartexplorer.android.launcher.shortcuts.Shortcuts
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
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
class LauncherViewModel(application: Application) : AndroidViewModel(application) {
    val repository: LayoutRepository = LayoutRepository.get(application)
    val catalog: AppCatalog = AppCatalog.get(application)
    val icons: IconLoader = IconLoader.get(application)
    val shortcuts: Shortcuts = Shortcuts(application)
    private val launcherApps: LauncherApps = application.getSystemService(LauncherApps::class.java)
    private val users: UserManager = application.getSystemService(UserManager::class.java)

    private val gridSize = MutableStateFlow(0 to 0)

    /** The layout as displayed in the current grid; the stored one is untouched by size changes (U35-19). */
    val display: StateFlow<HomeLayout> = combine(repository.layout, gridSize) { stored, size ->
        if (size.first <= 0 || size.second <= 0) stored else GridGeometry.placeLayout(stored, size.first, size.second)
    }.stateIn(viewModelScope, SharingStarted.Eagerly, repository.layout.value)

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
        get() = menu != null || moveTarget != null || openFolderId != null || editSheet || drawerOpen || query.isNotEmpty()

    fun setGrid(columns: Int, rows: Int) {
        gridSize.value = columns to rows
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
        if (editSheet) closeEditSheet()
        homeTick++
    }

    /** Closes the topmost layer; `false` when none is open. */
    fun closeTopLayer(): Boolean {
        when {
            menu != null -> menu = null
            moveTarget != null -> moveTarget = null
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

    companion object {
        /** Ids of items that are not in the layout yet (dragged out of the app list); a real id is given on the drop. */
        const val TRANSIENT_PREFIX = "drag:"
    }
}
