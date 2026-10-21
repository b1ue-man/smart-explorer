package app.smartexplorer.android.launcher.widgets

import android.appwidget.AppWidgetProviderInfo
import app.smartexplorer.android.launcher.model.Cell
import app.smartexplorer.android.launcher.model.GridGeometry
import app.smartexplorer.android.launcher.model.Geometry
import app.smartexplorer.android.launcher.model.Span
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

/** An area in dp (the pure twin of `android.util.SizeF`, so the JVM tests need no Android runtime). */
data class SizeDp(val width: Float, val height: Float)

/**
 * The grids of both orientations for the sizes a provider is told (U38-9): the [current] one, the grid of the
 * portrait and of the landscape orientation (the other one is computed with width and height swapped), and the
 * display density.
 */
data class SizeGrids(val current: Geometry, val portrait: Geometry, val landscape: Geometry, val density: Float) {
    companion object {
        /** [areaWidthDp] x [areaHeightDp] is the page area of the current orientation. */
        fun of(areaWidthDp: Float, areaHeightDp: Float, iconDp: Int, density: Float): SizeGrids {
            val current = GridGeometry.geometry(areaWidthDp, areaHeightDp, iconDp)
            val other = GridGeometry.geometry(areaHeightDp, areaWidthDp, iconDp)
            return if (areaWidthDp <= areaHeightDp) SizeGrids(current, current, other, density) else SizeGrids(current, other, current, density)
        }
    }
}

/**
 * The size rules of a widget on the home screen (U38-8, U38-9), pure Kotlin. Everything the platform tells
 * about a provider arrives as plain numbers, so [WidgetSizingTest] reaches all of it.
 */
object WidgetSizes {
    /**
     * The span a new widget starts with (U38-9): the provider's target cells from API 31 on (both above 0), else
     * the smallest span that holds `minWidth` x `minHeight` (pixels), at least 1 x 1, never beyond the grid.
     * [targetCellWidth] and [targetCellHeight] are 0 before API 31.
     */
    fun initialSpan(
        targetCellWidth: Int,
        targetCellHeight: Int,
        minWidthPx: Int,
        minHeightPx: Int,
        geometry: Geometry,
        density: Float,
    ): Span {
        val w: Int
        val h: Int
        if (targetCellWidth > 0 && targetCellHeight > 0) {
            w = targetCellWidth
            h = targetCellHeight
        } else {
            w = max(1, ceil(minWidthPx / (geometry.cellWidthDp * density)).toInt())
            h = max(1, ceil(minHeightPx / (geometry.cellHeightDp * density)).toInt())
        }
        return Span(w.coerceIn(1, geometry.columns), h.coerceIn(1, geometry.rows))
    }

    /** The area [span] covers in a grid of [geometry], in dp. */
    fun areaOf(span: Span, geometry: Geometry): SizeDp = SizeDp(span.w * geometry.cellWidthDp, span.h * geometry.cellHeightDp)

    /**
     * The two sizes the provider is told (U38-9): the area of [span] in the portrait and in the landscape grid.
     * The caller computes the grid of the other orientation with width and height swapped.
     */
    fun forSpan(span: Span, portrait: Geometry, landscape: Geometry): Pair<SizeDp, SizeDp> = areaOf(span, portrait) to areaOf(span, landscape)

    /**
     * The four numbers of the API 30 variant `updateAppWidgetSize(options, minWidth, minHeight, maxWidth, maxHeight)`:
     * minimum width and maximum height from portrait, maximum width and minimum height from landscape (U38-9).
     */
    fun fourValues(portrait: SizeDp, landscape: SizeDp): IntArray =
        intArrayOf(portrait.width.toInt(), landscape.height.toInt(), landscape.width.toInt(), portrait.height.toInt())

    /** The configuration activity runs when the provider has one and does not make it optional (API 31, U38-8). */
    fun needsConfiguration(hasConfigure: Boolean, widgetFeatures: Int, sdk: Int): Boolean {
        if (!hasConfigure) return false
        return !(sdk >= 31 && (widgetFeatures and AppWidgetProviderInfo.WIDGET_FEATURE_CONFIGURATION_OPTIONAL) != 0)
    }

    /** „Neu einrichten“ is offered for providers that are reconfigurable and have a configuration activity (U38-8). */
    fun canReconfigure(hasConfigure: Boolean, widgetFeatures: Int): Boolean =
        hasConfigure && (widgetFeatures and AppWidgetProviderInfo.WIDGET_FEATURE_RECONFIGURABLE) != 0
}

/** Smallest and largest span a widget may have, in cells (U38-9). */
data class ResizeLimits(val minW: Int, val maxW: Int, val minH: Int, val maxH: Int) {
    val horizontal: Boolean get() = minW != maxW
    val vertical: Boolean get() = minH != maxH
    val any: Boolean get() = horizontal || vertical
}

/** The four sides a widget can be pulled at. */
enum class Side { Left, Top, Right, Bottom }

/** Limits and the pulling of a widget's sides in whole cells (U38-9), pure Kotlin. */
object WidgetResizing {
    /**
     * Limits from the provider's resize data. [resizeMode] is `AppWidgetProviderInfo.resizeMode`; the `max...Px`
     * values are 0 before API 31 or when the provider names none; the `min...Px` values are 0 when it names none
     * (then 1 cell). A direction without its resize flag keeps the [current] span. The grid always limits.
     */
    fun limits(
        resizeMode: Int,
        current: Span,
        minResizeWidthPx: Int,
        minResizeHeightPx: Int,
        maxResizeWidthPx: Int,
        maxResizeHeightPx: Int,
        cellWidthPx: Float,
        cellHeightPx: Float,
        columns: Int,
        rows: Int,
    ): ResizeLimits {
        val (minW, maxW) = axis(resizeMode and AppWidgetProviderInfo.RESIZE_HORIZONTAL != 0, current.w, minResizeWidthPx, maxResizeWidthPx, cellWidthPx, columns)
        val (minH, maxH) = axis(resizeMode and AppWidgetProviderInfo.RESIZE_VERTICAL != 0, current.h, minResizeHeightPx, maxResizeHeightPx, cellHeightPx, rows)
        return ResizeLimits(minW, maxW, minH, maxH)
    }

    private fun axis(flexible: Boolean, current: Int, minPx: Int, maxPx: Int, cellPx: Float, count: Int): Pair<Int, Int> {
        if (!flexible) return current to current
        val lower = if (minPx <= 0) 1 else max(1, ceil(minPx / cellPx).toInt())
        val lowerInGrid = min(lower, count)
        val upper = if (maxPx > 0) max(lowerInGrid, floor(maxPx / cellPx).toInt()) else count
        return lowerInGrid to min(upper, count)
    }

    /**
     * Pulls [side] of the area ([cell], [span]) by [delta] cells (positive: right or down). The result stays
     * inside the grid and [limits] and covers no cell of [occupiedByOthers]; when the asked step does not fit,
     * the largest step in the same direction that does is used. `null` when nothing changes.
     */
    fun pull(
        cell: Cell,
        span: Span,
        side: Side,
        delta: Int,
        limits: ResizeLimits,
        occupiedByOthers: Set<Cell>,
        columns: Int,
        rows: Int,
    ): Pair<Cell, Span>? {
        if (delta == 0) return null
        val step = if (delta > 0) -1 else 1
        var d = delta
        while (d != 0) {
            val candidate = apply(cell, span, side, d)
            if (fits(candidate, limits, occupiedByOthers, columns, rows)) return candidate.takeIf { it != cell to span }
            d += step
        }
        return null
    }

    private fun apply(cell: Cell, span: Span, side: Side, d: Int): Pair<Cell, Span> = when (side) {
        Side.Left -> Cell(cell.x + d, cell.y) to Span(span.w - d, span.h)
        Side.Right -> cell to Span(span.w + d, span.h)
        Side.Top -> Cell(cell.x, cell.y + d) to Span(span.w, span.h - d)
        Side.Bottom -> cell to Span(span.w, span.h + d)
    }

    private fun fits(area: Pair<Cell, Span>, limits: ResizeLimits, taken: Set<Cell>, columns: Int, rows: Int): Boolean {
        val (cell, span) = area
        if (span.w !in limits.minW..limits.maxW || span.h !in limits.minH..limits.maxH) return false
        if (cell.x < 0 || cell.y < 0 || cell.x + span.w > columns || cell.y + span.h > rows) return false
        return GridGeometry.isFree(taken, cell, span)
    }
}
