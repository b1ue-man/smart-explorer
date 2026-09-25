package app.smartexplorer.android.ui.analytics

import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Squarified layout (Bruls et al.): areas proportional to the weights, covering the whole area. */
class TreemapTest {
    private val tolerance = 1e-3f

    @Test
    fun paperExampleFillsTheAreaProportionally() {
        val weights = listOf(6L, 6L, 4L, 3L, 2L, 2L, 1L)
        val cells = Treemap.squarify(weights, 6f, 4f)
        assertEquals(weights.size, cells.size)
        assertEquals(weights.indices.toSet(), cells.map { it.index }.toSet())
        val total = weights.sum().toFloat()
        cells.forEach { cell ->
            val expected = weights[cell.index] / total * 24f
            assertEquals("area of cell ${cell.index}", expected, cell.width * cell.height, tolerance)
            assertTrue("cell ${cell.index} inside", cell.left >= -tolerance && cell.top >= -tolerance)
            assertTrue("cell ${cell.index} inside", cell.left + cell.width <= 6f + tolerance && cell.top + cell.height <= 4f + tolerance)
        }
        assertEquals(24f, cells.sumOf { (it.width * it.height).toDouble() }.toFloat(), tolerance)
        for (i in cells.indices) {
            for (j in i + 1 until cells.size) {
                assertTrue("cells ${cells[i].index} and ${cells[j].index} overlap", overlap(cells[i], cells[j]) < tolerance)
            }
        }
        // Cells come in descending weight order.
        val ordered = cells.map { weights[it.index] }
        assertEquals(ordered.sortedDescending(), ordered)
    }

    @Test
    fun emptyAndNonPositiveInputsGetNoCells() {
        assertTrue(Treemap.squarify(emptyList(), 10f, 10f).isEmpty())
        assertTrue(Treemap.squarify(listOf(5L), 0f, 10f).isEmpty())
        val cells = Treemap.squarify(listOf(0L, 5L, -3L), 10f, 10f)
        assertEquals(listOf(1), cells.map { it.index })
        assertEquals(100f, cells.single().width * cells.single().height, tolerance)
        assertTrue(cells.single().contains(5f, 5f))
    }

    private fun overlap(a: TreemapCell, b: TreemapCell): Float {
        val w = min(a.left + a.width, b.left + b.width) - max(a.left, b.left)
        val h = min(a.top + a.height, b.top + b.height) - max(a.top, b.top)
        return if (w <= 0f || h <= 0f) 0f else abs(w * h)
    }
}
