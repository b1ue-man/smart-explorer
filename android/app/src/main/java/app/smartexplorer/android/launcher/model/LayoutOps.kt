package app.smartexplorer.android.launcher.model

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull

/** Where an item sits in a layout. */
sealed interface Location {
    data class Page(val index: Int) : Location

    data object Dock : Location

    data class Folder(val folderId: String) : Location
}

data class Found(val location: Location, val item: HomeItem)

/**
 * Changes to a [HomeLayout] (U35-22, U35-23), pure and free of Android types. Every function takes the
 * layout as it is displayed and returns the new one; nothing here touches the file.
 */
object LayoutOps {
    /** Every item of [layout], folder children included. */
    fun allItems(layout: HomeLayout): List<HomeItem> =
        (layout.pages.flatten() + layout.dock).flatMap { listOf(it) + it.children }

    fun newId(layout: HomeLayout): String = LayoutJson.newId(allItems(layout).map { it.id }.toSet())

    fun find(layout: HomeLayout, id: String): Found? {
        layout.pages.forEachIndexed { index, page ->
            for (item in page) {
                if (item.id == id) return Found(Location.Page(index), item)
                item.children.firstOrNull { it.id == id }?.let { return Found(Location.Folder(item.id), it) }
            }
        }
        for (item in layout.dock) {
            if (item.id == id) return Found(Location.Dock, item)
            item.children.firstOrNull { it.id == id }?.let { return Found(Location.Folder(item.id), it) }
        }
        return null
    }

    /** The item of [items] that covers [cell]. */
    fun itemAt(items: List<HomeItem>, cell: Cell): HomeItem? = items.firstOrNull {
        cell.x >= it.cell.x && cell.x < it.cell.x + it.span.w && cell.y >= it.cell.y && cell.y < it.cell.y + it.span.h
    }

    /** A folder with one child becomes that child on the folder's cell; one without children disappears (U35-23). */
    private fun normalize(items: List<HomeItem>): List<HomeItem> = items.mapNotNull { item ->
        if (!item.isFolder) {
            item
        } else {
            when (item.children.size) {
                0 -> null
                1 -> item.children[0].copy(cell = item.cell, span = Span(1, 1))
                else -> item
            }
        }
    }

    private fun mapAll(layout: HomeLayout, transform: (List<HomeItem>) -> List<HomeItem>): HomeLayout =
        layout.copy(pages = layout.pages.map(transform), dock = transform(layout.dock))

    /** Removes every item for which [predicate] holds, folder children included, then applies the folder rules. */
    fun removeWhere(layout: HomeLayout, predicate: (HomeItem) -> Boolean): HomeLayout = mapAll(layout) { items ->
        normalize(
            items.filterNot(predicate).map { item ->
                if (item.children.isEmpty()) item else item.copy(children = item.children.filterNot(predicate))
            },
        )
    }

    fun remove(layout: HomeLayout, id: String): HomeLayout = removeWhere(layout) { it.id == id }

    /**
     * Removes the apps and shortcuts of [packageName] in the profile [userSerial] (U35-29, `onPackageRemoved`) and
     * the widgets whose provider belongs to it (U38-6).
     */
    fun removePackage(layout: HomeLayout, packageName: String, userSerial: Long): HomeLayout = removeWhere(layout) {
        ((it.isApp || it.isShortcut) && it.packageName == packageName && it.userSerial == userSerial) ||
            (it.isWidget && it.providerPackage == packageName && it.userSerial == userSerial)
    }

    /** Key in [HomeLayout.extra] of the switch „Privaten Bereich in der App-Liste zeigen“ (U38-14). */
    const val SHOW_PRIVATE_SPACE = "showPrivateSpace"

    /** True unless the person switched it off; a missing or foreign value means on. */
    fun showPrivateSpace(layout: HomeLayout): Boolean = (layout.extra[SHOW_PRIVATE_SPACE] as? JsonPrimitive)?.booleanOrNull ?: true

    fun withShowPrivateSpace(layout: HomeLayout, on: Boolean): HomeLayout =
        layout.copy(extra = JsonObject(layout.extra + (SHOW_PRIVATE_SPACE to JsonPrimitive(on))))

    /** The widget ids of all widget items of [layout] (U38-6); the host frees the ones that disappear. */
    fun widgetIds(layout: HomeLayout): Set<Int> = allItems(layout).filter { it.isWidget }.mapNotNull { it.widgetId }.toSet()

    /** Replaces the item [id] by `transform(item)` wherever it is, without touching any cell. */
    fun updateItem(layout: HomeLayout, id: String, transform: (HomeItem) -> HomeItem): HomeLayout = replaceItem(layout, id, transform)

    /** Moves and resizes the item [id] on its page (U38-9); the caller has checked that the area is free. */
    fun setArea(layout: HomeLayout, id: String, cell: Cell, span: Span): HomeLayout = replaceItem(layout, id) { it.copy(cell = cell, span = span) }

    private fun replaceItem(layout: HomeLayout, id: String, transform: (HomeItem) -> HomeItem): HomeLayout = mapAll(layout) { items ->
        items.map { if (it.id == id) transform(it) else it }
    }

    /** Adds [item] to page [page] (creating pages up to it) at its cell, or to the dock when [page] is `null`. */
    fun insert(layout: HomeLayout, item: HomeItem, page: Int?): HomeLayout {
        if (page == null) return layout.copy(dock = layout.dock + item.copy(span = Span(1, 1)))
        val pages = layout.pages.toMutableList()
        while (pages.size <= page) pages.add(emptyList())
        pages[page] = pages[page] + item
        return layout.copy(pages = pages)
    }

    /** The folder rule for the target [target] (U35-23): a folder with the target and [dragged] on the target's cell. */
    fun createFolder(layout: HomeLayout, target: HomeItem, dragged: HomeItem): HomeLayout {
        val folder = HomeItem(
            id = newId(layout),
            kind = Kinds.FOLDER,
            cell = target.cell,
            span = Span(1, 1),
            attrs = JsonObject(mapOf("name" to JsonPrimitive(HomeItem.FOLDER_NAME))),
            children = listOf(target.copy(cell = Cell(0, 0), span = Span(1, 1)), dragged.copy(cell = Cell(0, 0), span = Span(1, 1))),
        )
        return replaceItem(layout, target.id) { folder }
    }

    /** Appends [dragged] to the folder [folderId]. */
    fun addToFolder(layout: HomeLayout, folderId: String, dragged: HomeItem): HomeLayout =
        replaceItem(layout, folderId) { it.copy(children = it.children + dragged.copy(cell = Cell(0, 0), span = Span(1, 1))) }

    /** Renames a folder; an empty name becomes „Ordner“. */
    fun renameFolder(layout: HomeLayout, folderId: String, name: String): HomeLayout =
        replaceItem(layout, folderId) { it.withAttr("name", JsonPrimitive(name.trim().ifEmpty { HomeItem.FOLDER_NAME })) }

    /**
     * Dissolves a folder (U35-27 „Ordner auflösen“ and „Entfernen“): the folder disappears, each child goes to
     * the free cell nearest to the folder's cell on the same page (in the dock: the nearest free dock place),
     * and onto a later or new page when that page has no room. [layout] must be the displayed layout, whose
     * `grid` carries the current columns and rows.
     */
    fun dissolveFolder(layout: HomeLayout, folderId: String): HomeLayout {
        val found = find(layout, folderId) ?: return layout
        val folder = found.item
        var result = mapAll(layout) { items -> items.filterNot { it.id == folderId } }
        for (child in folder.children) result = placeNear(result, child.copy(span = Span(1, 1)), found.location, folder.cell)
        return result
    }

    /** Puts a 1x1 [item] on the free cell nearest to [near] in [location]; a later page or a new one when there is none. */
    fun placeNear(layout: HomeLayout, item: HomeItem, location: Location, near: Cell): HomeLayout {
        val columns = layout.grid.columns.coerceAtLeast(1)
        val rows = layout.grid.rows.coerceAtLeast(1)
        if (location is Location.Dock) {
            val taken = GridGeometry.occupied(layout.dock)
            val cell = GridGeometry.nearestFree(taken, Cell(near.x, 0), Span(1, 1), columns, 1)
            if (cell != null) return layout.copy(dock = layout.dock + item.copy(cell = cell))
            return addAnywhere(layout, item.copy(cell = Cell(0, 0)), 0)
        }
        val page = (location as? Location.Page)?.index ?: 0
        val cell = GridGeometry.nearestFree(GridGeometry.occupied(layout.pages.getOrElse(page) { emptyList() }), near, Span(1, 1), columns, rows)
        if (cell != null) return insert(layout, item.copy(cell = cell), page)
        return addAnywhere(layout, item.copy(cell = near), page + 1)
    }

    /** Drops pages without items, except the first (U35-22). */
    fun pruneEmptyPages(layout: HomeLayout): HomeLayout =
        layout.copy(pages = layout.pages.filterIndexed { index, page -> index == 0 || page.isNotEmpty() })

    /**
     * Puts [item] on the first page with room, starting at [startPage], near its cell; a new page when none
     * has room (`LayoutRepository.addItem`, U35-36). Items that do not fit the grid at all are clamped.
     */
    fun addAnywhere(layout: HomeLayout, item: HomeItem, startPage: Int): HomeLayout {
        val columns = layout.grid.columns.coerceAtLeast(1)
        val rows = layout.grid.rows.coerceAtLeast(1)
        val span = Span(item.span.w.coerceIn(1, columns), item.span.h.coerceIn(1, rows))
        var page = startPage.coerceIn(0, (layout.pages.size - 1).coerceAtLeast(0))
        while (true) {
            val items = layout.pages.getOrElse(page) { emptyList() }
            val cell = GridGeometry.nearestFree(GridGeometry.occupied(items), item.cell, span, columns, rows)
            if (cell != null) return insert(layout, item.copy(cell = cell, span = span), page)
            page++
        }
    }
}
