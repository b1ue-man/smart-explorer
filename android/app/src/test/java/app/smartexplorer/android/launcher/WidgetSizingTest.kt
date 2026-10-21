package app.smartexplorer.android.launcher

import android.appwidget.AppWidgetProviderInfo
import app.smartexplorer.android.launcher.model.Cell
import app.smartexplorer.android.launcher.model.GridGeometry
import app.smartexplorer.android.launcher.model.Span
import app.smartexplorer.android.launcher.widgets.ResizeLimits
import app.smartexplorer.android.launcher.widgets.Side
import app.smartexplorer.android.launcher.widgets.SizeDp
import app.smartexplorer.android.launcher.widgets.SizeGrids
import app.smartexplorer.android.launcher.widgets.WidgetResizing
import app.smartexplorer.android.launcher.widgets.WidgetSizes
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The size rules of widgets (U38-8, U38-9), without Android. */
class WidgetSizingTest {
    private val phone = GridGeometry.geometry(360f, 600f, 56)

    @Test
    fun theStartSpanFollowsTheProviderTargetOrItsMinimumSize() {
        // 5 x 6 cells of 72 x 100 dp at density 2.
        assertEquals(5, phone.columns)
        assertEquals(6, phone.rows)
        // Target cells win from API 31 on.
        assertEquals(Span(2, 2), WidgetSizes.initialSpan(2, 2, 999, 999, phone, 2f))
        // One value of the target is 0: it does not count.
        assertEquals(Span(1, 1), WidgetSizes.initialSpan(2, 0, 10, 10, phone, 2f))
        // 110 dp x 110 dp = 220 px: ceil(220 / (72 * 2)) = 2 columns, ceil(220 / (100 * 2)) = 2 rows.
        assertEquals(Span(2, 2), WidgetSizes.initialSpan(0, 0, 220, 220, phone, 2f))
        // Nothing asked: 1 x 1. Larger than the grid: the grid.
        assertEquals(Span(1, 1), WidgetSizes.initialSpan(0, 0, 0, 0, phone, 2f))
        assertEquals(Span(phone.columns, phone.rows), WidgetSizes.initialSpan(0, 0, 100_000, 100_000, phone, 2f))
        assertEquals(Span(phone.columns, 1), WidgetSizes.initialSpan(99, 1, 0, 0, phone, 2f))
    }

    @Test
    fun theTwoSizesAreTheAreaInBothOrientationsAndTheFourValuesPickTheRightEdges() {
        val portrait = SizeGrids.of(360f, 600f, 56, 2f)
        assertEquals(portrait.current, portrait.portrait)
        // The other orientation swaps width and height of the page area.
        assertEquals(GridGeometry.geometry(600f, 360f, 56), portrait.landscape)
        val landscape = SizeGrids.of(600f, 360f, 56, 2f)
        assertEquals(landscape.current, landscape.landscape)
        assertEquals(portrait.portrait, landscape.portrait)
        assertEquals(portrait.landscape, landscape.landscape)

        val (p, l) = WidgetSizes.forSpan(Span(2, 3), portrait.portrait, portrait.landscape)
        assertEquals(2 * portrait.portrait.cellWidthDp, p.width, 0.001f)
        assertEquals(3 * portrait.portrait.cellHeightDp, p.height, 0.001f)
        assertEquals(2 * portrait.landscape.cellWidthDp, l.width, 0.001f)
        assertEquals(3 * portrait.landscape.cellHeightDp, l.height, 0.001f)
        // API 30: minimum width and maximum height from portrait, maximum width and minimum height from landscape.
        assertArrayEquals(
            intArrayOf(100, 60, 220, 180),
            WidgetSizes.fourValues(SizeDp(100.9f, 180.2f), SizeDp(220.5f, 60.7f)),
        )
    }

    @Test
    fun configurationRunsUnlessTheProviderMakesItOptionalFromApi31() {
        val optional = AppWidgetProviderInfo.WIDGET_FEATURE_CONFIGURATION_OPTIONAL
        val reconfigurable = AppWidgetProviderInfo.WIDGET_FEATURE_RECONFIGURABLE
        assertFalse(WidgetSizes.needsConfiguration(false, 0, 36))
        assertTrue(WidgetSizes.needsConfiguration(true, 0, 30))
        assertTrue(WidgetSizes.needsConfiguration(true, reconfigurable, 36))
        assertFalse(WidgetSizes.needsConfiguration(true, optional, 31))
        assertFalse(WidgetSizes.needsConfiguration(true, optional or reconfigurable, 36))
        // The bit means nothing before API 31.
        assertTrue(WidgetSizes.needsConfiguration(true, optional, 30))
        assertTrue(WidgetSizes.canReconfigure(true, reconfigurable))
        assertFalse(WidgetSizes.canReconfigure(true, 0))
        assertFalse(WidgetSizes.canReconfigure(false, reconfigurable))
    }

    private val both = AppWidgetProviderInfo.RESIZE_HORIZONTAL or AppWidgetProviderInfo.RESIZE_VERTICAL

    @Test
    fun theLimitsComeFromTheProviderAndAlwaysFromTheGrid() {
        val cell = 200f
        // minResize 40 dp at density 2.6 = 104 px: one cell. 0 means one cell as well.
        assertEquals(ResizeLimits(1, 5, 1, 8), WidgetResizing.limits(both, Span(2, 2), 104, 0, 0, 0, cell, cell, 5, 8))
        // minResize 450 px = 3 cells; max 900 px = 4 cells (API 31); the grid cuts the maximum to 4 columns.
        assertEquals(ResizeLimits(3, 4, 1, 8), WidgetResizing.limits(both, Span(3, 2), 450, 0, 900, 0, cell, cell, 4, 8))
        // A maximum below the minimum cannot go below it.
        assertEquals(ResizeLimits(3, 3, 1, 8), WidgetResizing.limits(both, Span(3, 2), 450, 0, 300, 0, cell, cell, 5, 8))
        // The minimum larger than the grid is the grid.
        assertEquals(ResizeLimits(5, 5, 1, 8), WidgetResizing.limits(both, Span(5, 2), 9000, 0, 0, 0, cell, cell, 5, 8))
        // Without a resize flag the direction keeps the current span.
        val horizontal = WidgetResizing.limits(AppWidgetProviderInfo.RESIZE_HORIZONTAL, Span(2, 3), 0, 0, 0, 0, cell, cell, 5, 8)
        assertEquals(ResizeLimits(1, 5, 3, 3), horizontal)
        assertTrue(horizontal.horizontal)
        assertFalse(horizontal.vertical)
        val none = WidgetResizing.limits(AppWidgetProviderInfo.RESIZE_NONE, Span(2, 3), 0, 0, 0, 0, cell, cell, 5, 8)
        assertFalse(none.any)
    }

    @Test
    fun pullingASideMovesWholeCellsInsideLimitsGridAndFreeArea() {
        val limits = ResizeLimits(1, 4, 1, 4)
        val none = emptySet<Cell>()
        // Right side one cell out; left side one cell out moves the origin.
        assertEquals(Cell(1, 1) to Span(3, 2), WidgetResizing.pull(Cell(1, 1), Span(2, 2), Side.Right, 1, limits, none, 5, 5))
        assertEquals(Cell(0, 1) to Span(3, 2), WidgetResizing.pull(Cell(1, 1), Span(2, 2), Side.Left, -1, limits, none, 5, 5))
        assertEquals(Cell(1, 2) to Span(2, 1), WidgetResizing.pull(Cell(1, 1), Span(2, 2), Side.Top, 1, limits, none, 5, 5))
        assertEquals(Cell(1, 1) to Span(2, 1), WidgetResizing.pull(Cell(1, 1), Span(2, 2), Side.Bottom, -1, limits, none, 5, 5))
        // Too far: the largest step that fits (the grid ends at column 5, the limit at 4 cells).
        assertEquals(Cell(1, 1) to Span(4, 2), WidgetResizing.pull(Cell(1, 1), Span(2, 2), Side.Right, 9, limits, none, 5, 5))
        assertEquals(Cell(0, 1) to Span(3, 2), WidgetResizing.pull(Cell(1, 1), Span(2, 2), Side.Left, -9, ResizeLimits(1, 3, 1, 4), none, 5, 5))
        // Another item in the way stops the step before it.
        val blocker = setOf(Cell(4, 1))
        assertEquals(Cell(1, 1) to Span(3, 2), WidgetResizing.pull(Cell(1, 1), Span(2, 2), Side.Right, 3, limits, blocker, 5, 5))
        // Below the lower limit and no change are refused.
        assertNull(WidgetResizing.pull(Cell(1, 1), Span(1, 1), Side.Right, -1, limits, none, 5, 5))
        assertNull(WidgetResizing.pull(Cell(1, 1), Span(2, 2), Side.Right, 0, limits, none, 5, 5))
        assertNull(WidgetResizing.pull(Cell(1, 1), Span(2, 2), Side.Right, 1, ResizeLimits(2, 2, 1, 4), none, 5, 5))
    }
}
