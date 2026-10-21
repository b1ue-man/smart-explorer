package app.smartexplorer.android.launcher.model

/** Result of dropping an item (U35-22). */
sealed interface DropOutcome {
    /** The new layout (not yet placed again; the caller places it, see `GridGeometry.placeLayout`). */
    data class Applied(val layout: HomeLayout) : DropOutcome

    /** The page has no free area of the size: the item returns, the person is told „Auf dieser Seite ist kein Platz.“ */
    data object NoSpace : DropOutcome
}

/**
 * The rules for dropping an item on a page or in the dock (U35-22, U35-23), pure. [layout] is the displayed
 * layout; [dragged] may already be in it (it is taken out first, folder rules applied) or come from outside
 * (app list, shortcut menu). [merge] says that the finger stayed over the target for `FOLDER_DWELL_MS` in its
 * inner area.
 */
object DropPlanner {
    const val NO_SPACE_TEXT = "Auf dieser Seite ist kein Platz."

    fun dropOnPage(layout: HomeLayout, dragged: HomeItem, page: Int, cell: Cell, merge: Boolean): DropOutcome {
        val columns = layout.grid.columns.coerceAtLeast(1)
        val rows = layout.grid.rows.coerceAtLeast(1)
        val base = if (LayoutOps.find(layout, dragged.id) != null) LayoutOps.remove(layout, dragged.id) else layout
        val span = Span(dragged.span.w.coerceIn(1, columns), dragged.span.h.coerceIn(1, rows))
        val items = base.pages.getOrElse(page) { emptyList() }
        val taken = GridGeometry.occupied(items)
        val target = Cell(cell.x.coerceIn(0, columns - span.w), cell.y.coerceIn(0, rows - span.h))

        val occupant = LayoutOps.itemAt(items, cell)
        if (occupant != null && merge) mergeInto(base, occupant, dragged)?.let { return DropOutcome.Applied(it) }
        val free = if (GridGeometry.isFree(taken, target, span)) target else GridGeometry.nearestFree(taken, target, span, columns, rows)
        return if (free == null) {
            DropOutcome.NoSpace
        } else {
            DropOutcome.Applied(LayoutOps.insert(base, dragged.copy(cell = free, span = span), page))
        }
    }

    /** The dock holds items of 1x1 only, one row of `columns` places. */
    fun dropOnDock(layout: HomeLayout, dragged: HomeItem, index: Int, merge: Boolean): DropOutcome {
        val columns = layout.grid.columns.coerceAtLeast(1)
        if (dragged.span != Span(1, 1) || !(dragged.canBeInFolder || dragged.isFolder)) return DropOutcome.NoSpace
        val base = if (LayoutOps.find(layout, dragged.id) != null) LayoutOps.remove(layout, dragged.id) else layout
        val taken = GridGeometry.occupied(base.dock)
        val target = Cell(index.coerceIn(0, columns - 1), 0)
        val occupant = LayoutOps.itemAt(base.dock, target)
        if (occupant != null && merge) mergeInto(base, occupant, dragged)?.let { return DropOutcome.Applied(it) }
        val free = if (target !in taken) target else GridGeometry.nearestFree(taken, target, Span(1, 1), columns, 1)
        return if (free == null) DropOutcome.NoSpace else DropOutcome.Applied(base.copy(dock = base.dock + dragged.copy(cell = free)))
    }

    /** Folder rules: onto a folder, an app or shortcut is appended; app on app makes a folder. */
    private fun mergeInto(layout: HomeLayout, occupant: HomeItem, dragged: HomeItem): HomeLayout? = when {
        !dragged.canBeInFolder -> null
        occupant.isFolder -> LayoutOps.addToFolder(layout, occupant.id, dragged)
        occupant.canBeInFolder -> LayoutOps.createFolder(layout, occupant, dragged)
        else -> null
    }
}
