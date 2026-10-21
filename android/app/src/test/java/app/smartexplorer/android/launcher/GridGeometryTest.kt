package app.smartexplorer.android.launcher

import app.smartexplorer.android.launcher.model.Cell
import app.smartexplorer.android.launcher.model.GridGeometry
import app.smartexplorer.android.launcher.model.GridSpec
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.HomeLayout
import app.smartexplorer.android.launcher.model.Kinds
import app.smartexplorer.android.launcher.model.Span
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Grid, size formula and re-assignment (U35-21), without Android. */
class GridGeometryTest {
    private fun item(id: String, x: Int, y: Int, w: Int = 1, h: Int = 1) = HomeItem(id, Kinds.APP, Cell(x, y), Span(w, h))

    @Test
    fun theFormulaGivesTheSpecifiedCounts() {
        // (width, height) -> columns/rows for icon sizes 48, 56, 64 (cell at least max(icon+16, 72) by icon+36 dp).
        val expected = mapOf(
            (360f to 800f) to listOf(5 to 9, 5 to 8, 4 to 8),
            (411f to 914f) to listOf(5 to 10, 5 to 9, 5 to 9),
            (600f to 960f) to listOf(8 to 11, 8 to 10, 7 to 9),
            (840f to 960f) to listOf(11 to 11, 11 to 10, 10 to 9),
        )
        for ((size, counts) in expected) {
            GridGeometry.ICON_SIZES.forEachIndexed { index, icon ->
                val geometry = GridGeometry.geometry(size.first, size.second, icon)
                assertEquals("columns of ${size.first}x${size.second} at $icon", counts[index].first, geometry.columns)
                assertEquals("rows of ${size.first}x${size.second} at $icon", counts[index].second, geometry.rows)
                assertEquals(size.first / geometry.columns, geometry.cellWidthDp, 0.001f)
                assertEquals(size.second / geometry.rows, geometry.cellHeightDp, 0.001f)
                assertTrue(geometry.cellWidthDp >= GridGeometry.cellMinWidthDp(icon))
                assertTrue(geometry.cellHeightDp >= GridGeometry.cellMinHeightDp(icon))
            }
        }
        // A tiny area still has one cell.
        assertEquals(1, GridGeometry.geometry(10f, 10f, 56).columns)
        assertEquals(1, GridGeometry.geometry(10f, 10f, 56).rows)
    }

    @Test
    fun shrinkingTheGridKeepsEveryItemAndAddsPages() {
        val items = (0 until 6).flatMap { y -> (0 until 5).map { x -> item("i$x$y", x, y) } }
        val placed = GridGeometry.place(listOf(items), columns = 4, rows = 5)
        assertEquals(2, placed.size)
        assertEquals(20, placed[0].size)
        assertEquals(10, placed[1].size)
        assertEquals(items.map { it.id }.toSet(), placed.flatten().map { it.id }.toSet())
        for (page in placed) {
            val cells = page.map { it.cell }
            assertEquals(cells.size, cells.toSet().size)
            assertTrue(cells.all { it.x in 0..3 && it.y in 0..4 })
        }
        // The items inside the smaller grid did not move.
        assertEquals(Cell(2, 3), placed[0].first { it.id == "i23" }.cell)
        // Growing the grid again leaves the stored cells in place.
        val back = GridGeometry.place(listOf(items), columns = 5, rows = 6)
        assertEquals(1, back.size)
        assertEquals(Cell(4, 5), back[0].first { it.id == "i45" }.cell)
    }

    @Test
    fun overlappingAndOutsideItemsMoveToFreeAreasInReadingOrder() {
        val page = listOf(item("a", 0, 0), item("b", 0, 0), item("c", 9, 9), item("w", 1, 0, w = 9, h = 1))
        val placed = GridGeometry.place(listOf(page), columns = 3, rows = 2)
        val byId = placed.flatten().associateBy { it.id }
        assertEquals(Cell(0, 0), byId.getValue("a").cell)
        // The wide item is cut to the grid and stays where it fits (x 1 with width 3 does not fit: it moves).
        assertEquals(Span(3, 1), byId.getValue("w").span)
        assertEquals(4, byId.size)
        assertEquals(1, placed.size)
        val cells = placed.flatMapIndexed { p, items -> items.flatMap { i -> (0 until i.span.h).flatMap { dy -> (0 until i.span.w).map { dx -> Triple(p, i.cell.x + dx, i.cell.y + dy) } } } }
        assertEquals(cells.size, cells.toSet().size)
    }

    @Test
    fun nearestFreeBreaksTiesInReadingOrderAndFailsWithoutRoom() {
        val occupied = setOf(Cell(1, 1))
        assertEquals(Cell(1, 0), GridGeometry.nearestFree(occupied, Cell(1, 1), Span(1, 1), 3, 3))
        assertEquals(Cell(0, 0), GridGeometry.nearestFree(emptySet(), Cell(0, 0), Span(1, 1), 3, 3))
        assertEquals(Cell(1, 1), GridGeometry.nearestFree(setOf(Cell(0, 0), Cell(1, 0), Cell(2, 0)), Cell(1, 0), Span(1, 1), 3, 2))
        val all = (0 until 2).flatMap { y -> (0 until 2).map { x -> Cell(x, y) } }.toSet()
        assertNull(GridGeometry.nearestFree(all, Cell(0, 0), Span(1, 1), 2, 2))
        assertNull(GridGeometry.nearestFree(emptySet(), Cell(0, 0), Span(3, 1), 2, 2))
    }

    @Test
    fun theDockKeepsColumnsAndReturnsWhatDoesNotFit() {
        val dock = listOf(item("a", 0, 0), item("b", 0, 0), item("c", 7, 0))
        val (placed, overflow) = GridGeometry.placeDock(dock, columns = 2)
        assertEquals(listOf("a", "b"), placed.map { it.id })
        assertEquals(listOf(Cell(0, 0), Cell(1, 0)), placed.map { it.cell })
        assertEquals(listOf("c"), overflow.map { it.id })
        val layout = HomeLayout(GridSpec(2, 2, 56), listOf(emptyList()), dock)
        val shown = GridGeometry.placeLayout(layout, 2, 2)
        assertEquals(2, shown.dock.size)
        assertEquals(listOf("c"), shown.pages.flatten().map { it.id })
    }
}
