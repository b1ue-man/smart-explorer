package app.smartexplorer.android.launcher.ui

import android.view.ViewConfiguration
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.launcher.LauncherViewModel
import app.smartexplorer.android.launcher.MenuSource
import app.smartexplorer.android.launcher.MenuTarget
import app.smartexplorer.android.launcher.apps.AppCatalog
import app.smartexplorer.android.launcher.badges.Badges
import app.smartexplorer.android.launcher.gestures.homeGestures
import app.smartexplorer.android.launcher.model.Cell
import app.smartexplorer.android.launcher.model.GridGeometry
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.LayoutOps
import app.smartexplorer.android.launcher.model.Location
import app.smartexplorer.android.launcher.se.SeRoomPopup
import app.smartexplorer.android.launcher.se.SeSeeder
import app.smartexplorer.android.launcher.se.SeSheet
import app.smartexplorer.android.launcher.se.SeStatusLine
import app.smartexplorer.android.launcher.se.SeVirtual
import app.smartexplorer.android.launcher.wallpaper.Wallpaper
import app.smartexplorer.android.launcher.widgets.WidgetPicker
import app.smartexplorer.android.launcher.widgets.WidgetResizeOverlay
import kotlinx.coroutines.delay

/**
 * The home screen (DSK-M10, U35-17 to U35-32): pages with a grid, the dock, page dots with „Alle Apps“, and the
 * layers above them (app list, folder, menu, settings sheet, accessibility dialog). The page area, the dock and
 * the drop zones are laid out here in plain pixels, so [resolveHover] and the long press find the item under the
 * finger without asking the composition. All drags — from the grid, the app list, a folder or the menu — end in
 * [DragHost.end] below.
 */
@Composable
fun HomeScreen(vm: LauncherViewModel) {
    val context = LocalContext.current
    val density = LocalDensity.current
    val haptics = LocalHapticFeedback.current
    val view = LocalView.current
    val direction = LocalLayoutDirection.current
    val display by vm.display.collectAsStateWithLifecycle()
    val apps by vm.catalog.apps.collectAsStateWithLifecycle()
    val profiles by vm.catalog.profiles.collectAsStateWithLifecycle()
    val shortcutInfos by vm.shortcutInfos.collectAsStateWithLifecycle()
    val dots by Badges.dots.collectAsStateWithLifecycle()
    val showPrivate by vm.showPrivateSpace.collectAsStateWithLifecycle()
    val seUi by vm.se.ui.collectAsStateWithLifecycle()
    // A locked private space draws nothing and keeps its cells (U38-14).
    val hiddenUsers = remember(profiles) { profiles.filter { it.isHidden }.map { it.userSerial }.toSet() }
    val drag = remember { DragState() }
    val insets = WindowInsets.systemBars.union(WindowInsets.displayCutout).asPaddingValues()
    val slop = remember(context) { ViewConfiguration.get(context).scaledTouchSlop.toFloat() }
    var mergeArmed by remember { mutableStateOf(false) }

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val iconDp = display.grid.iconDp
        val left = insets.calculateLeftPadding(direction)
        val right = insets.calculateRightPadding(direction)
        val top = insets.calculateTopPadding()
        val bottom = insets.calculateBottomPadding()
        val dockHeight = GridGeometry.dockHeightDp(iconDp).dp
        val indicator = GridGeometry.INDICATOR_DP.dp
        val areaW = (maxWidth - left - right).coerceAtLeast(1.dp)
        val areaH = (maxHeight - top - bottom - dockHeight - indicator).coerceAtLeast(1.dp)
        val geometry = GridGeometry.geometry(areaW.value, areaH.value, iconDp)
        LaunchedEffect(geometry.columns, geometry.rows) { vm.setGrid(geometry.columns, geometry.rows) }
        LaunchedEffect(areaW.value, areaH.value, iconDp) { vm.setGeometry(areaW.value, areaH.value, iconDp) }
        val metrics = with(density) {
            val cellW = areaW.toPx() / geometry.columns
            HomeMetrics(
                left = left.toPx(),
                top = top.toPx(),
                width = areaW.toPx(),
                height = areaH.toPx(),
                columns = geometry.columns,
                rows = geometry.rows,
                cellW = cellW,
                cellH = areaH.toPx() / geometry.rows,
                dockTop = (top + areaH + indicator).toPx(),
                dockHeight = dockHeight.toPx(),
                zoneHeight = HomeMetrics.zoneHeightPx(density.density),
                edgeWidth = cellW / 2,
            )
        }
        val screenW = with(density) { maxWidth.toPx() }
        val screenH = with(density) { maxHeight.toPx() }

        val dragging = drag.active && drag.moved
        val showNewPage = dragging || vm.editSheet
        val pagerState = rememberPagerState(pageCount = { display.pages.size + if (showNewPage) 1 else 0 })
        // The binding rule C3.3 (a): is a se element on the visible page or in the dock? (U40-7)
        val visibleHasSe = (display.pages.getOrNull(pagerState.currentPage).orEmpty() + display.dock).any { it.isSe }
        LaunchedEffect(visibleHasSe, pagerState.currentPage) { vm.se.setVisibleHasSe(visibleHasSe) }
        LaunchedEffect(Unit) { vm.se.messages.collect { vm.showToast(it) } }
        // „Gerät hinzufügen“ and „Raum beitreten“ stand on page 1 while the lists are empty; they are computed, not stored (U40-9).
        val pagesShown = if (SeVirtual.wanted(seUi.data, seUi.fresh, SeSeeder.isSeeded(display))) {
            SeVirtual.inject(display.pages, geometry.columns, geometry.rows)
        } else {
            display.pages
        }
        val itemContext = remember(vm, iconDp, apps, shortcutInfos, dots, hiddenUsers) {
            ItemContext(vm, iconDp, apps.associateBy { it.key }, shortcutInfos, dots, hiddenUsers)
        }
        val metricsNow by rememberUpdatedState(metrics)
        val displayNow by rememberUpdatedState(display)
        val showNewNow by rememberUpdatedState(showNewPage)
        val mergeNow by rememberUpdatedState(mergeArmed)
        val grab = remember { Grab() }

        fun hoverAt(at: Offset): Hover {
            val item = drag.item
            val entry = if (item != null && item.isApp) vm.entryFor(item) else null
            return resolveHover(at, metricsNow, pagerState.currentPage, displayNow, isUninstallable(entry), if (showNewNow) displayNow.pages.size else null)
        }

        // A widget follows the finger with its middle, so the cell under the finger is the middle of its area (U38-9).
        fun anchorCell(item: HomeItem, under: Cell): Cell =
            if (item.isWidget) Cell(under.x - (item.span.w - 1) / 2, under.y - (item.span.h - 1) / 2) else under

        // Nothing is folded into an item that is hidden: its cell only looks empty (U38-14).
        fun hiddenOccupant(id: String?): Boolean = id != null && LayoutOps.find(displayNow, id)?.item?.let { itemContext.isHidden(it) } == true

        fun onWidget(at: Offset): Boolean {
            val m = metricsNow
            if (!m.contains(at)) return false
            val items = displayNow.pages.getOrNull(pagerState.currentPage) ?: return false
            return LayoutOps.itemAt(items, m.cellAt(at))?.isWidget == true
        }

        fun anchorOf(item: HomeItem, onDock: Boolean): Offset {
            val m = metricsNow
            return if (onDock) {
                Offset(m.left + (item.cell.x + 0.5f) * m.cellW, m.dockTop + m.dockHeight / 2)
            } else {
                Offset(m.left + (item.cell.x + item.span.w / 2f) * m.cellW, m.top + (item.cell.y + item.span.h / 2f) * m.cellH)
            }
        }

        val host = remember(vm, drag, pagerState) {
            object : DragHost {
                override fun begin(item: HomeItem, at: Offset, origin: DragOrigin, source: MenuSource, anchor: Offset) {
                    haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                    drag.begin(item, at, origin)
                    grab.source = source
                    grab.anchor = anchor
                }

                override fun move(at: Offset) = drag.move(at, hoverAt(at), edgeOf(at, metricsNow), slop)

                override fun end(at: Offset, cancelled: Boolean) {
                    val item = drag.item ?: return
                    val moved = drag.moved
                    val origin = drag.origin
                    val hover = hoverAt(at)
                    val merge = mergeNow
                    drag.finish()
                    if (!cancelled && !moved) {
                        // A long press that stayed in place opens the menu of the item.
                        if (origin != DragOrigin.Menu) vm.menu = MenuTarget(item, grab.source, grab.anchor)
                        return
                    }
                    if (!cancelled) {
                        drop(item, origin, hover, merge, at)
                        // Pages the drag left empty disappear when the edit mode ends (U35-22).
                        if (!vm.editSheet) vm.pruneEmptyPages()
                    }
                    if (moved) {
                        when (origin) {
                            DragOrigin.Drawer -> vm.closeDrawer()
                            DragOrigin.Folder -> vm.closeFolder()
                            DragOrigin.Menu -> vm.menu = null
                            DragOrigin.Home -> Unit
                        }
                    }
                }

                private fun drop(item: HomeItem, origin: DragOrigin, hover: Hover, merge: Boolean, at: Offset) {
                    when (hover) {
                        is Hover.OnCell -> vm.dropOnPage(item, hover.page, anchorCell(item, hover.cell), merge && hover.inner && !hiddenOccupant(hover.occupantId))
                        is Hover.OnDock -> vm.dropOnDock(item, hover.index, merge && hover.inner && !hiddenOccupant(hover.occupantId))
                        is Hover.OnNewPage -> vm.dropOnPage(item, hover.page, anchorCell(item, metricsNow.cellAt(at)), merge = false)
                        Hover.Remove -> if (origin == DragOrigin.Home || origin == DragOrigin.Folder) vm.removeItem(item.id)
                        Hover.Uninstall -> vm.entryFor(item)?.let { uninstall(context, it) }
                        Hover.None -> Unit
                    }
                }
            }
        }

        itemContext.dragHost = host
        itemContext.isDragActive = { drag.active }

        // The finger rests on a target's inner area: after the dwell time a drop makes or fills a folder.
        val mergeKey = when (val hover = drag.hover) {
            is Hover.OnCell -> if (hover.inner && hover.occupantId != null) hover.page to hover.occupantId else null
            is Hover.OnDock -> if (hover.inner && hover.occupantId != null) -1 to hover.occupantId else null
            else -> null
        }
        LaunchedEffect(mergeKey, drag.active) {
            mergeArmed = false
            if (drag.active && mergeKey != null) {
                delay(FOLDER_DWELL_MS)
                mergeArmed = true
            }
        }
        // The finger stays at a page edge: the page changes after the dwell time, and again while it stays.
        LaunchedEffect(drag.edge, pagerState.currentPage, drag.active) {
            if (drag.active && drag.edge != 0) {
                delay(EDGE_DWELL_MS)
                val target = pagerState.currentPage + drag.edge
                if (target in 0 until pagerState.pageCount) pagerState.animateScrollToPage(target)
            }
        }
        LaunchedEffect(pagerState) { snapshotFlow { pagerState.currentPage }.collect { vm.currentPage = it } }
        LaunchedEffect(vm.homeTick) { if (vm.homeTick > 0) pagerState.animateScrollToPage(0) }
        LaunchedEffect(pagerState, display.pages.size) {
            snapshotFlow { pagerState.currentPage + pagerState.currentPageOffsetFraction }.collect { position ->
                val pages = display.pages.size
                Wallpaper.setOffsets(context, view.windowToken, pages, if (pages > 1) position / (pages - 1) else 0f)
            }
        }
        // The folder was emptied or removed while its layer was open.
        val openFolder = vm.openFolderId?.let { id -> LayoutOps.find(display, id)?.item?.takeIf { it.isFolder } }
        LaunchedEffect(vm.openFolderId, openFolder == null) { if (vm.openFolderId != null && openFolder == null) vm.closeFolder() }
        val hiddenDrag = drag.active && drag.moved

        // ---- the home screen proper -------------------------------------------------------------------
        Box(
            Modifier.fillMaxSize()
                .longPressDrag(
                    key = Unit,
                    onLongPress = { local ->
                        when {
                            drag.active || (vm.hasOpenLayer && !vm.editSheet) -> false
                            else -> {
                                val m = metricsNow
                                val onDock = m.inDock(local)
                                val page = pagerState.currentPage
                                val found = when {
                                    onDock -> LayoutOps.itemAt(displayNow.dock, Cell(m.dockIndexAt(local), 0))
                                    m.contains(local) -> displayNow.pages.getOrNull(page)?.let { LayoutOps.itemAt(it, m.cellAt(local)) }
                                    else -> null
                                }
                                if (found != null && itemContext.isHidden(found)) {
                                    // The reserved cell of a locked private space is neither an item nor an empty cell.
                                    false
                                } else if (found != null) {
                                    host.begin(found, local, DragOrigin.Home, MenuSource.Home, anchorOf(found, onDock))
                                    true
                                } else {
                                    if (!onDock && m.contains(local) && page < displayNow.pages.size) {
                                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                        vm.openEditSheet()
                                    }
                                    false
                                }
                            }
                        }
                    },
                    onMove = { host.move(it) },
                    onEnd = { last, cancelled -> host.end(last, cancelled) },
                )
                .homeGestures(
                    enabled = !vm.hasOpenLayer && !drag.active,
                    onSwipeUp = { vm.openDrawer() },
                    blockedAt = { onWidget(it) },
                ),
        ) {
            val excluded = if (drag.origin == DragOrigin.Home) drag.item?.id else null
            Box(Modifier.offset(left, top).size(areaW, areaH)) {
                HomePager(pagerState, pagesShown, showNewPage, metrics, itemContext, excluded, !drag.active)
                // The handles of the widget chosen with „Größe ändern“ (U38-9), on its page.
                val resizing = vm.resizeId?.let { id -> LayoutOps.find(display, id) }
                LaunchedEffect(vm.resizeId, resizing == null) { if (vm.resizeId != null && resizing == null) vm.endResize() }
                val resizeLocation = resizing?.location
                if (resizing != null && resizeLocation is Location.Page && resizeLocation.index == pagerState.currentPage) {
                    WidgetResizeOverlay(resizing.item, display.pages[resizeLocation.index], metrics, itemContext) { vm.endResize() }
                }
            }
            Box(Modifier.offset(left, top + areaH).size(areaW, indicator)) {
                PageIndicator(pagerState.currentPage.coerceAtMost(display.pages.size - 1), display.pages.size, { vm.openDrawer() })
            }
            Box(Modifier.offset(left, top + areaH + indicator).size(areaW, dockHeight)) {
                Dock(display.dock, metrics, itemContext, excluded)
            }
        }

        // ---- layers ------------------------------------------------------------------------------------
        if (vm.drawerOpen) {
            AppDrawer(
                apps = AppCatalog.shown(apps, showPrivate),
                profiles = profiles,
                context = itemContext,
                host = host,
                iconDp = iconDp,
                insets = insets,
                hidden = hiddenDrag && drag.origin == DragOrigin.Drawer,
                showPrivate = showPrivate,
                onClose = { vm.closeDrawer() },
            )
        }
        openFolder?.let { folder ->
            FolderPopup(folder, itemContext, host, metrics.columns, hiddenDrag && drag.origin == DragOrigin.Folder) { vm.closeFolder() }
        }
        // The room popup closes when its room left the data (like a folder that was emptied).
        val openRoom = vm.openRoomId
        LaunchedEffect(openRoom, seUi.data) {
            if (openRoom != null && seUi.data?.rooms?.none { it.profileId == openRoom } == true) vm.openRoomId = null
        }
        if (openRoom != null) SeRoomPopup(vm, openRoom, metrics.columns) { vm.openRoomId = null }
        if (vm.editSheet) {
            EditSheet(vm, iconDp, Modifier.align(Alignment.BottomCenter).padding(bottom = bottom, start = left, end = right))
        }
        if (vm.seSheet) {
            SeSheet(vm, onClose = { vm.closeSeSheet() }, modifier = Modifier.padding(bottom = bottom))
        }
        SeStatusLine(vm, visibleHasSe && !vm.hasOpenLayer, Modifier.align(Alignment.BottomCenter).padding(bottom = bottom + dockHeight + indicator + 8.dp, start = 24.dp, end = 24.dp))
        if (vm.widgetPicker) {
            WidgetPicker(vm, onClose = { vm.closeWidgetPicker() }, modifier = Modifier.padding(bottom = bottom))
        }
        vm.menu?.let { target ->
            Box(Modifier.fillMaxSize().alpha(if (hiddenDrag && drag.origin == DragOrigin.Menu) 0f else 1f)) {
                ItemMenu(target, MenuContext(vm, target.source, context), host, screenW, screenH) { vm.menu = null }
            }
        }
        val draggedEntry = drag.item?.takeIf { it.isApp }?.let { vm.entryFor(it) }
        DropZones(drag, metrics, isUninstallable(draggedEntry))
        DragPreview(drag, metrics, itemContext)
        vm.moveTarget?.let { item ->
            MoveDialog(
                pages = display.pages.size,
                columns = metrics.columns,
                rows = metrics.rows,
                onMove = { page, column, row ->
                    vm.moveTo(item, page, column, row)
                    vm.moveTarget = null
                },
                onDismiss = { vm.moveTarget = null },
            )
        }
        vm.toast?.let { text ->
            LaunchedEffect(text) {
                delay(TOAST_MS)
                vm.clearToast()
            }
            Surface(
                Modifier.align(Alignment.BottomCenter).padding(bottom = bottom + dockHeight + indicator + 16.dp, start = 24.dp, end = 24.dp),
                shape = MaterialTheme.shapes.medium,
                color = MaterialTheme.colorScheme.inverseSurface,
            ) {
                Text(text, Modifier.padding(horizontal = 16.dp, vertical = 12.dp), color = MaterialTheme.colorScheme.inverseOnSurface)
            }
        }
    }
}

/** The source and anchor of the press that began a drag, kept for the menu a motionless long press opens. */
private class Grab {
    var source: MenuSource = MenuSource.Home
    var anchor: Offset = Offset.Zero
}

/** How long a message stays; a UX value. */
private const val TOAST_MS = 3_000L
