package app.smartexplorer.android.ui.analytics

import kotlin.math.max
import kotlin.math.min

/** One placed rectangle; [index] is the position of its weight in the input list. */
internal data class TreemapCell(val index: Int, val left: Float, val top: Float, val width: Float, val height: Float) {
    fun contains(x: Float, y: Float): Boolean = x >= left && x < left + width && y >= top && y < top + height
}

/**
 * Squarified treemap layout (Bruls, Huizing, van Wijk 2000): rectangles with areas proportional to
 * their weights and aspect ratios close to 1. Pure Kotlin, independent of Compose.
 */
internal object Treemap {
    private const val EPSILON = 1e-6

    /**
     * Lays out [weights] in a [width] × [height] area. Weights ≤ 0 get no cell. Cells come in
     * descending weight order; their areas add up to the whole area.
     */
    fun squarify(weights: List<Long>, width: Float, height: Float): List<TreemapCell> {
        if (width <= 0f || height <= 0f) return emptyList()
        val items = weights.withIndex().filter { it.value > 0 }.sortedByDescending { it.value }
        val total = items.sumOf { it.value.toDouble() }
        if (items.isEmpty() || total <= 0.0) return emptyList()
        val scale = width.toDouble() * height.toDouble() / total
        val areas = items.map { IndexedValue(it.index, it.value * scale) }

        val cells = ArrayList<TreemapCell>(areas.size)
        val free = Area(0.0, 0.0, width.toDouble(), height.toDouble())
        val row = ArrayList<IndexedValue<Double>>()
        var next = 0
        while (next < areas.size) {
            val side = min(free.w, free.h)
            if (side <= EPSILON) break
            val candidate = areas[next]
            if (row.isEmpty() || worst(row, candidate.value, side) <= worst(row, null, side)) {
                row += candidate
                next++
            } else {
                place(row, free, cells)
                row.clear()
            }
        }
        if (row.isNotEmpty()) place(row, free, cells)
        return cells
    }

    /** Remaining free rectangle; shrinks as rows are placed. */
    private class Area(var x: Double, var y: Double, var w: Double, var h: Double)

    /** Worst aspect ratio of [row] (plus [extra], if given) laid along a side of length [side]. */
    private fun worst(row: List<IndexedValue<Double>>, extra: Double?, side: Double): Double {
        var sum = extra ?: 0.0
        var largest = extra ?: 0.0
        var smallest = extra ?: Double.MAX_VALUE
        row.forEach {
            sum += it.value
            largest = max(largest, it.value)
            smallest = min(smallest, it.value)
        }
        if (sum <= 0.0 || smallest <= 0.0) return Double.MAX_VALUE
        val side2 = side * side
        val sum2 = sum * sum
        return max(side2 * largest / sum2, sum2 / (side2 * smallest))
    }

    /** Places [row] along the shorter side of [free] and removes that strip from it. */
    private fun place(row: List<IndexedValue<Double>>, free: Area, cells: MutableList<TreemapCell>) {
        val sum = row.sumOf { it.value }
        if (free.w >= free.h) {
            // Column at the left edge, items stacked top to bottom.
            val columnWidth = min(sum / free.h, free.w)
            var y = free.y
            row.forEach {
                val h = it.value / columnWidth
                cells += TreemapCell(it.index, free.x.toFloat(), y.toFloat(), columnWidth.toFloat(), h.toFloat())
                y += h
            }
            free.x += columnWidth
            free.w = max(0.0, free.w - columnWidth)
        } else {
            // Row at the top edge, items left to right.
            val rowHeight = min(sum / free.w, free.h)
            var x = free.x
            row.forEach {
                val w = it.value / rowHeight
                cells += TreemapCell(it.index, x.toFloat(), free.y.toFloat(), w.toFloat(), rowHeight.toFloat())
                x += w
            }
            free.y += rowHeight
            free.h = max(0.0, free.h - rowHeight)
        }
    }
}
