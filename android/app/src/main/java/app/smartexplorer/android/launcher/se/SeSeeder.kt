package app.smartexplorer.android.launcher.se

import app.smartexplorer.android.launcher.bridge.DevicesData
import app.smartexplorer.android.launcher.model.Cell
import app.smartexplorer.android.launcher.model.GridGeometry
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.HomeLayout
import app.smartexplorer.android.launcher.model.Kinds
import app.smartexplorer.android.launcher.model.LayoutOps
import app.smartexplorer.android.launcher.model.Span
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** One se element the person can put on the home screen: a [kind] of the four se kinds and its `attrs.target`. */
data class SeEntry(val kind: String, val target: String?, val name: String) {
    val key: String get() = "$kind|${target.orEmpty()}"
}

/**
 * The first fill of the home screen with se elements and the sheet „se-Elemente“ (U40-10), pure Kotlin on a
 * [HomeLayout]. Seeding happens once, with the first current `DEVICES` ([SEEDED] in `extra`); devices that appear
 * later are never added on their own.
 */
object SeSeeder {
    /** Key in [HomeLayout.extra] (U35-18, U35-20). A missing key means „not seeded“. */
    const val SEEDED = "seSeeded"

    fun isSeeded(layout: HomeLayout): Boolean = (layout.extra[SEEDED] as? JsonPrimitive)?.content == "true"

    /** Every element of [data] in the order of the first fill: devices, rooms, vault sites, „Alle Bildschirme“. */
    fun entries(data: DevicesData): List<SeEntry> =
        data.devices.map { SeEntry(Kinds.SE_DEVICE, it.location.trimEnd('/'), it.name) } +
            data.rooms.map { SeEntry(Kinds.SE_ROOM, it.profileId, it.name) } +
            data.vaults.map { SeEntry(Kinds.SE_VAULT, SeActions.vaultLocation(it.site), it.label) } +
            SeEntry(Kinds.SE_SCREENS, null, SeViews.ALL_SCREENS)

    /** The elements of [layout] as entries, to tell which are on a page already. */
    fun present(layout: HomeLayout): Set<String> =
        LayoutOps.allItems(layout).filter { it.isSe }.map { SeEntry(it.kind, it.attr("target"), "").key }.toSet()

    /** The first fill: all entries on page 1 and after it, in reading order, then [SEEDED] true. */
    fun seed(layout: HomeLayout, data: DevicesData): HomeLayout = markSeeded(add(layout, entries(data), 0))

    /** [entries] not on a page yet, put on the first free cells of [page] and the following pages. */
    fun add(layout: HomeLayout, entries: List<SeEntry>, page: Int): HomeLayout {
        var result = layout
        val have = present(layout).toMutableSet()
        for (entry in entries) {
            if (!have.add(entry.key)) continue
            result = placeInReadingOrder(result, itemOf(result, entry), page)
        }
        return result
    }

    /**
     * Puts the 1x1 [item] on the first free cell of [startPage] in reading order (row by row, left to right), else of the
     * next page, else of a new page. (`LayoutOps.addAnywhere` takes the cell nearest to a point, which is not reading order.)
     */
    private fun placeInReadingOrder(layout: HomeLayout, item: HomeItem, startPage: Int): HomeLayout {
        val columns = layout.grid.columns.coerceAtLeast(1)
        val rows = layout.grid.rows.coerceAtLeast(1)
        var page = startPage.coerceIn(0, (layout.pages.size - 1).coerceAtLeast(0))
        while (true) {
            val cell = SeVirtual.firstFreeInReadingOrder(GridGeometry.occupied(layout.pages.getOrElse(page) { emptyList() }), columns, rows)
            if (cell != null) return LayoutOps.insert(layout, item.copy(cell = cell), page)
            page++
        }
    }

    fun markSeeded(layout: HomeLayout): HomeLayout = layout.copy(extra = JsonObject(layout.extra + (SEEDED to JsonPrimitive(true))))

    /**
     * Brings the stored `name` of each se element in line with the current data, so a tile whose target vanished can
     * still show its name. Returns [layout] itself when nothing changes (nothing is written then).
     */
    fun syncNames(layout: HomeLayout, data: DevicesData): HomeLayout {
        val names = entries(data).associate { it.key to it.name }
        var result = layout
        for (item in LayoutOps.allItems(layout).filter { it.isSe }) {
            val name = names[SeEntry(item.kind, item.attr("target"), "").key] ?: continue
            if (name.isNotEmpty() && item.attr("name") != name) {
                result = LayoutOps.updateItem(result, item.id) { it.withAttr("name", JsonPrimitive(name)) }
            }
        }
        return result
    }

    private fun itemOf(layout: HomeLayout, entry: SeEntry): HomeItem {
        val attrs = LinkedHashMap<String, JsonPrimitive>()
        entry.target?.let { attrs["target"] = JsonPrimitive(it) }
        if (entry.name.isNotEmpty()) attrs["name"] = JsonPrimitive(entry.name)
        return HomeItem(id = LayoutOps.newId(layout), kind = entry.kind, cell = Cell(0, 0), span = Span(1, 1), attrs = JsonObject(attrs))
    }
}

/**
 * The two tiles „Gerät hinzufügen“ and „Raum beitreten“ of an empty device list (U40-9): computed whenever the page
 * is drawn, put on the first free cells of page 1 and never stored.
 */
object SeVirtual {
    const val ADD_DEVICE_ID = "se-virtual:add-device"
    const val JOIN_ROOM_ID = "se-virtual:join-room"

    /** True when the groups belong on the screen: current data, no device and no room, and the first fill is done. */
    fun wanted(data: DevicesData?, fresh: Boolean, seeded: Boolean): Boolean = fresh && seeded && data != null && data.noDevicesOrRooms

    /** The first cell of a [columns] by [rows] page that is not in [taken], row by row and left to right. */
    fun firstFreeInReadingOrder(taken: Set<Cell>, columns: Int, rows: Int): Cell? {
        for (y in 0 until rows) for (x in 0 until columns) if (Cell(x, y) !in taken) return Cell(x, y)
        return null
    }

    fun inject(pages: List<List<HomeItem>>, columns: Int, rows: Int): List<List<HomeItem>> {
        if (pages.isEmpty() || columns < 1 || rows < 1) return pages
        var first = pages[0]
        for ((id, kind) in listOf(ADD_DEVICE_ID to Kinds.SE_ADD_DEVICE, JOIN_ROOM_ID to Kinds.SE_JOIN_ROOM)) {
            val cell = firstFreeInReadingOrder(GridGeometry.occupied(first), columns, rows) ?: break
            first = first + HomeItem(id = id, kind = kind, cell = cell)
        }
        return listOf(first) + pages.drop(1)
    }
}
