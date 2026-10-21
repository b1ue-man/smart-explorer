package app.smartexplorer.android.launcher.model

import kotlin.math.abs
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

/** Grid for a page area (U35-21): [columns] by [rows] cells of [cellWidthDp] by [cellHeightDp]. */
data class Geometry(
    val columns: Int,
    val rows: Int,
    val cellWidthDp: Float,
    val cellHeightDp: Float,
)

/**
 * Raster, size formula and re-assignment (U35-21), pure Kotlin. Nothing here deletes an item: after a
 * size change (rotation, folding) every item stays visible and reachable.
 */
object GridGeometry {
    /** Symbol sizes the formula is tested for (U35-30). */
    val ICON_SIZES = listOf(48, 56, 64)

    /** Height of the dock row beside the cell height: one cell and 8 dp. */
    const val DOCK_PADDING_DP = 8f

    /** Height of the row with the page dots. */
    const val INDICATOR_DP = 32f

    fun cellMinWidthDp(iconDp: Int): Float = max(iconDp + 16, 72).toFloat()

    fun cellMinHeightDp(iconDp: Int): Float = (iconDp + 36).toFloat()

    fun dockHeightDp(iconDp: Int): Float = cellMinHeightDp(iconDp) + DOCK_PADDING_DP

    /** [widthDp] and [heightDp] are the page area: without system bar insets, dock and page dots. */
    fun geometry(widthDp: Float, heightDp: Float, iconDp: Int): Geometry {
        val columns = max(1, floor(widthDp / cellMinWidthDp(iconDp)).toInt())
        val rows = max(1, floor(heightDp / cellMinHeightDp(iconDp)).toInt())
        return Geometry(columns, rows, widthDp / columns, heightDp / rows)
    }

    /** Cells occupied by [items]. */
    fun occupied(items: List<HomeItem>): Set<Cell> {
        val cells = HashSet<Cell>()
        for (item in items) {
            for (dy in 0 until item.span.h) for (dx in 0 until item.span.w) cells.add(Cell(item.cell.x + dx, item.cell.y + dy))
        }
        return cells
    }

    private fun fits(cell: Cell, span: Span, columns: Int, rows: Int): Boolean =
        cell.x >= 0 && cell.y >= 0 && cell.x + span.w <= columns && cell.y + span.h <= rows

    /** True when no cell of the area is in [occupied]. */
    fun isFree(occupied: Set<Cell>, cell: Cell, span: Span): Boolean {
        for (dy in 0 until span.h) for (dx in 0 until span.w) if (Cell(cell.x + dx, cell.y + dy) in occupied) return false
        return true
    }

    /**
     * The free area of [span] closest to [target] (Manhattan distance, reading order on a tie), or `null`
     * when the page has none.
     */
    fun nearestFree(occupied: Set<Cell>, target: Cell, span: Span, columns: Int, rows: Int): Cell? {
        var best: Cell? = null
        var bestDistance = Int.MAX_VALUE
        for (y in 0..rows - span.h) {
            for (x in 0..columns - span.w) {
                val cell = Cell(x, y)
                if (!isFree(occupied, cell, span)) continue
                val distance = abs(x - target.x) + abs(y - target.y)
                // Reading order is the iteration order, so a later cell must be strictly closer.
                if (distance < bestDistance) {
                    best = cell
                    bestDistance = distance
                }
            }
        }
        return best
    }

    /** First free area of [span] in reading order, or `null`. */
    fun firstFree(occupied: Set<Cell>, span: Span, columns: Int, rows: Int): Cell? = nearestFree(occupied, Cell(0, 0), span, columns, rows)

    private fun clampSpan(item: HomeItem, columns: Int, rows: Int): HomeItem {
        val span = Span(min(item.span.w, columns).coerceAtLeast(1), min(item.span.h, rows).coerceAtLeast(1))
        return if (span == item.span) item else item.copy(span = span)
    }

    /**
     * Places [pages] into a grid of [columns] by [rows] (U35-21): spans are limited to the grid; in the
     * order (page, row, column) of the stored cell an item stays when it lies inside the grid and covers
     * no cell taken before; all others, in the same order, take the first free area of their page, else
     * of a later page, else of a new page at the end. [extra] items (a dock that no longer fits) are
     * placed like items of the first page that did not fit. Deterministic, deletes nothing.
     */
    fun place(pages: List<List<HomeItem>>, columns: Int, rows: Int, extra: List<HomeItem> = emptyList()): List<List<HomeItem>> {
        val cols = max(1, columns)
        val rws = max(1, rows)
        val result = ArrayList<MutableList<HomeItem>>()
        val taken = ArrayList<MutableSet<Cell>>()
        repeat(max(1, pages.size)) {
            result.add(ArrayList<HomeItem>())
            taken.add(HashSet<Cell>())
        }
        val pending = ArrayList<Pair<Int, HomeItem>>()
        pages.forEachIndexed { index, items ->
            val ordered = items.map { clampSpan(it, cols, rws) }
                .withIndex()
                .sortedWith(compareBy({ it.value.cell.y }, { it.value.cell.x }, { it.index }))
                .map { it.value }
            for (item in ordered) {
                if (fits(item.cell, item.span, cols, rws) && isFree(taken[index], item.cell, item.span)) {
                    taken[index].addAll(occupied(listOf(item)))
                    result[index].add(item)
                } else {
                    pending.add(index to item)
                }
            }
        }
        extra.forEach { pending.add(0 to clampSpan(it, cols, rws)) }
        for ((start, item) in pending) {
            var page = start
            while (true) {
                if (page >= result.size) {
                    result.add(ArrayList<HomeItem>())
                    taken.add(HashSet<Cell>())
                }
                val cell = firstFree(taken[page], item.span, cols, rws)
                if (cell != null) {
                    val placed = item.copy(cell = cell)
                    taken[page].addAll(occupied(listOf(placed)))
                    result[page].add(placed)
                    break
                }
                page++
            }
        }
        return result
    }

    /**
     * The dock row: items keep their column when it is in range and free; the others take the first free
     * place. Items that do not fit into [columns] places are returned second (the caller puts them on a page).
     */
    fun placeDock(dock: List<HomeItem>, columns: Int): Pair<List<HomeItem>, List<HomeItem>> {
        val cols = max(1, columns)
        val slots = arrayOfNulls<HomeItem>(cols)
        val rest = ArrayList<HomeItem>()
        for (item in dock.withIndex().sortedWith(compareBy({ it.value.cell.x }, { it.index })).map { it.value }) {
            val x = item.cell.x
            if (x in 0 until cols && slots[x] == null) slots[x] = item.copy(cell = Cell(x, 0), span = Span(1, 1)) else rest.add(item)
        }
        val overflow = ArrayList<HomeItem>()
        for (item in rest) {
            val free = slots.indexOfFirst { it == null }
            if (free >= 0) slots[free] = item.copy(cell = Cell(free, 0), span = Span(1, 1)) else overflow.add(item.copy(cell = Cell(0, 0)))
        }
        return slots.filterNotNull() to overflow
    }

    /** The layout as it is displayed in a grid of [columns] by [rows] (U35-19); the stored layout is not changed by this. */
    fun placeLayout(layout: HomeLayout, columns: Int, rows: Int): HomeLayout {
        val (dock, overflow) = placeDock(layout.dock, columns)
        val pages = place(layout.pages, columns, rows, overflow)
        return layout.copy(grid = layout.grid.copy(columns = max(1, columns), rows = max(1, rows)), pages = pages, dock = dock)
    }
}
