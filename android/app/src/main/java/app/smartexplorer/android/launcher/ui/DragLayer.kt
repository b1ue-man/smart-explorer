package app.smartexplorer.android.launcher.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.changedToUpIgnoreConsumed
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.unit.IntSize
import app.smartexplorer.android.launcher.MenuSource
import app.smartexplorer.android.launcher.model.Cell
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.HomeLayout
import app.smartexplorer.android.launcher.model.LayoutOps
import kotlin.math.roundToInt

/** Dwell at the page edge before the page changes (U35-22); a UX constant, not a limit. */
const val EDGE_DWELL_MS = 500L

/** Dwell over a target before dropping makes a folder (U35-22). */
const val FOLDER_DWELL_MS = 400L

/** Height of the drop zones at the top of the page area. */
private const val ZONE_HEIGHT_DP = 72

/** Where the dragged item came from. */
enum class DragOrigin { Home, Drawer, Folder, Menu }

/**
 * The page area, the dock and the zones in root pixels, as laid out by the home screen. Cells are `cellW` by
 * `cellH`; the dock row has the same columns.
 */
data class HomeMetrics(
    val left: Float,
    val top: Float,
    val width: Float,
    val height: Float,
    val columns: Int,
    val rows: Int,
    val cellW: Float,
    val cellH: Float,
    val dockTop: Float,
    val dockHeight: Float,
    val zoneHeight: Float,
    val edgeWidth: Float,
) {
    fun contains(pointer: Offset): Boolean = pointer.x >= left && pointer.x < left + width && pointer.y >= top && pointer.y < top + height

    fun inDock(pointer: Offset): Boolean = pointer.x >= left && pointer.x < left + width && pointer.y >= dockTop && pointer.y < dockTop + dockHeight

    /** The cell under [pointer] in the page area, clamped into the grid. */
    fun cellAt(pointer: Offset): Cell = Cell(
        ((pointer.x - left) / cellW).toInt().coerceIn(0, columns - 1),
        ((pointer.y - top) / cellH).toInt().coerceIn(0, rows - 1),
    )

    fun dockIndexAt(pointer: Offset): Int = ((pointer.x - left) / cellW).toInt().coerceIn(0, columns - 1)

    /** True when [pointer] is in the middle half of its cell, in both directions (U35-22). */
    fun isInner(pointer: Offset, cell: Cell, dock: Boolean): Boolean {
        val fx = (pointer.x - left) / cellW - cell.x
        val fy = if (dock) (pointer.y - dockTop) / dockHeight else (pointer.y - top) / cellH - cell.y
        return fx in 0.25f..0.75f && fy in 0.25f..0.75f
    }

    companion object {
        fun zoneHeightPx(density: Float): Float = ZONE_HEIGHT_DP * density
    }
}

/** What lies under the finger. */
sealed interface Hover {
    data object None : Hover

    data class OnCell(val page: Int, val cell: Cell, val inner: Boolean, val occupantId: String?) : Hover

    data class OnDock(val index: Int, val inner: Boolean, val occupantId: String?) : Hover

    /** The „Neue Seite“ to the right of the last page. */
    data class OnNewPage(val page: Int) : Hover

    data object Remove : Hover

    data object Uninstall : Hover
}

/** Which page edge the finger touches: −1 left, +1 right, 0 none. */
fun edgeOf(pointer: Offset, metrics: HomeMetrics): Int = when {
    pointer.y < metrics.top || pointer.y >= metrics.top + metrics.height -> 0
    pointer.x < metrics.left + metrics.edgeWidth -> -1
    pointer.x >= metrics.left + metrics.width - metrics.edgeWidth -> 1
    else -> 0
}

/**
 * Resolves the finger position to a drop target (U35-22), pure: the zones at the top win, then the dock, then
 * the page area. [page] is the page the pager shows, [newPageIndex] the index of the „Neue Seite“.
 */
fun resolveHover(
    pointer: Offset,
    metrics: HomeMetrics,
    page: Int,
    display: HomeLayout,
    canUninstall: Boolean,
    newPageIndex: Int?,
): Hover {
    if (pointer.y < metrics.top + metrics.zoneHeight) {
        val half = metrics.left + metrics.width / 2
        return if (canUninstall && pointer.x >= half) Hover.Uninstall else Hover.Remove
    }
    if (metrics.inDock(pointer)) {
        val index = metrics.dockIndexAt(pointer)
        val occupant = LayoutOps.itemAt(display.dock, Cell(index, 0))
        return Hover.OnDock(index, metrics.isInner(pointer, Cell(index, 0), dock = true), occupant?.id)
    }
    if (metrics.contains(pointer)) {
        if (newPageIndex != null && page == newPageIndex) return Hover.OnNewPage(page)
        val cell = metrics.cellAt(pointer)
        val occupant = LayoutOps.itemAt(display.pages.getOrElse(page) { emptyList() }, cell)
        return Hover.OnCell(page, cell, metrics.isInner(pointer, cell, dock = false), occupant?.id)
    }
    return Hover.None
}

/**
 * The drag in progress, shared by the home screen, the app list and the folder popup. [moved] turns true when
 * the finger left the touch slop: a long press without movement opens the menu on release instead of dropping.
 */
@Stable
class DragState {
    var item by mutableStateOf<HomeItem?>(null)
        private set
    var pointer by mutableStateOf(Offset.Zero)
        private set
    var origin by mutableStateOf(DragOrigin.Home)
        private set
    var moved by mutableStateOf(false)
        private set
    var hover by mutableStateOf<Hover>(Hover.None)
        private set

    /** Page edge the finger touches: −1 left, +1 right, 0 none (see [edgeOf]). */
    var edge by mutableStateOf(0)
        private set
    private var start = Offset.Zero

    val active: Boolean get() = item != null

    fun begin(item: HomeItem, at: Offset, origin: DragOrigin) {
        this.item = item
        this.origin = origin
        pointer = at
        start = at
        moved = false
        hover = Hover.None
        edge = 0
    }

    fun move(to: Offset, hover: Hover, edge: Int, slopPx: Float) {
        pointer = to
        this.hover = hover
        this.edge = edge
        if (!moved && (to - start).getDistance() > slopPx) moved = true
    }

    fun finish() {
        item = null
        moved = false
        hover = Hover.None
        edge = 0
    }
}

/**
 * Long press, then drag (U35-22). Events are watched in the initial pass, so a press that stays put is not
 * taken away from the children (tap, pager); once the long press fires and [onLongPress] says it starts a
 * drag, every further event of the pointer is consumed here. Positions are local to the node. [onEnd] gets
 * the last position and `true` when the gesture was cancelled.
 */
@Composable
fun Modifier.longPressDrag(
    key: Any?,
    onLongPress: (Offset) -> Boolean,
    onMove: (Offset) -> Unit,
    onEnd: (Offset, Boolean) -> Unit,
): Modifier {
    val longPress = rememberUpdatedState(onLongPress)
    val move = rememberUpdatedState(onMove)
    val end = rememberUpdatedState(onEnd)
    return pointerInput(key) {
        val timeout = viewConfiguration.longPressTimeoutMillis
        val slop = viewConfiguration.touchSlop
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
            var released = false
            val timedOut = withTimeoutOrNull(timeout) {
                while (!released) {
                    val event = awaitPointerEvent(PointerEventPass.Initial)
                    val change = event.changes.firstOrNull { it.id == down.id }
                    released = change == null || change.changedToUpIgnoreConsumed() ||
                        (change.position - down.position).getDistance() > slop
                }
                true
            } == null
            if (!timedOut || !longPress.value(down.position)) return@awaitEachGesture
            var last = down.position
            while (true) {
                val event = awaitPointerEvent(PointerEventPass.Initial)
                val change = event.changes.firstOrNull { it.id == down.id }
                if (change == null) {
                    end.value(last, true)
                    return@awaitEachGesture
                }
                change.consume()
                last = change.position
                if (change.changedToUpIgnoreConsumed()) {
                    end.value(last, false)
                    return@awaitEachGesture
                }
                move.value(last)
            }
        }
    }
}

/** What a drag source reports to the home screen; positions are in root pixels. */
interface DragHost {
    /** A long press on [item] begins. [anchor] is the item's centre, where a menu would open. */
    fun begin(item: HomeItem, at: Offset, origin: DragOrigin, source: MenuSource, anchor: Offset)

    fun move(at: Offset)

    fun end(at: Offset, cancelled: Boolean)
}

/**
 * Makes a node a source of drags to the home screen: the app list, the folder popup and the shortcut rows of
 * the menu use this. [payload] gives the item to drag, or `null` to ignore the press.
 */
@Composable
fun Modifier.dragSource(
    host: DragHost,
    key: Any?,
    origin: DragOrigin,
    source: MenuSource,
    payload: () -> HomeItem?,
): Modifier {
    var position by remember { mutableStateOf(Offset.Zero) }
    var size by remember { mutableStateOf(IntSize.Zero) }
    return onGloballyPositioned {
        position = it.positionInRoot()
        size = it.size
    }.longPressDrag(
        key = key,
        onLongPress = { local ->
            val item = payload()
            if (item == null) {
                false
            } else {
                host.begin(item, position + local, origin, source, position + Offset(size.width / 2f, size.height / 2f))
                true
            }
        },
        onMove = { host.move(position + it) },
        onEnd = { last, cancelled -> host.end(position + last, cancelled) },
    )
}

/** The dragged item follows the finger, half transparent (U35-22). */
@Composable
fun DragPreview(drag: DragState, metrics: HomeMetrics, context: ItemContext) {
    val item = drag.item ?: return
    if (!drag.moved) return
    val density = LocalDensity.current
    val width = with(density) { metrics.cellW.toDp() }
    val height = with(density) { metrics.cellH.toDp() }
    Box(
        Modifier
            .offset { IntOffset((drag.pointer.x - metrics.cellW / 2).roundToInt(), (drag.pointer.y - metrics.cellH / 2).roundToInt()) }
            .size(width, height)
            .alpha(0.6f),
    ) { ItemContent(item, context) }
}

/**
 * The drop zones at the top while an item is dragged (U35-22): „Entfernen“, and „Deinstallieren“ only for an
 * app that can be uninstalled.
 */
@Composable
fun DropZones(drag: DragState, metrics: HomeMetrics, canUninstall: Boolean) {
    if (!drag.active || !drag.moved) return
    val density = LocalDensity.current
    val top = with(density) { metrics.top.toDp() }
    Row(
        Modifier.offset(y = top).fillMaxWidth().height(with(density) { metrics.zoneHeight.toDp() }),
    ) {
        Zone("Entfernen", drag.hover == Hover.Remove, Modifier.weight(1f))
        if (canUninstall) Zone("Deinstallieren", drag.hover == Hover.Uninstall, Modifier.weight(1f))
    }
}

@Composable
private fun androidx.compose.foundation.layout.RowScope.Zone(label: String, highlighted: Boolean, modifier: Modifier) {
    Box(
        modifier
            .fillMaxHeight()
            .background(if (highlighted) MaterialTheme.colorScheme.primary.copy(alpha = 0.85f) else Color.Black.copy(alpha = 0.55f))
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Text(
            label,
            color = if (highlighted) MaterialTheme.colorScheme.onPrimary else Color.White,
            style = MaterialTheme.typography.titleSmall,
            textAlign = TextAlign.Center,
        )
    }
}
