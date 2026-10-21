package app.smartexplorer.android.launcher.model

import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull

/** Cell of the grid, counted from the top left (U35-18). */
data class Cell(val x: Int, val y: Int)

/** Width and height in cells. */
data class Span(val w: Int, val h: Int)

/** Kinds this package draws itself; U38 adds `widget`, U40 `seDevice`, `seRoom`, `seVault`, `seScreens` (U35-31). */
object Kinds {
    const val APP = "app"
    const val SHORTCUT = "shortcut"
    const val FOLDER = "folder"
}

/**
 * One element of the home screen (U35-18). [attrs] holds every other key of the entry (`component`,
 * `user`, `shortcutId`, `provider`, `target`, unknown ones), so a kind this version does not know
 * survives a round trip. [children] only exists for folders; a child has no cell of its own.
 */
data class HomeItem(
    val id: String,
    val kind: String,
    val cell: Cell = Cell(0, 0),
    val span: Span = Span(1, 1),
    val attrs: JsonObject = JsonObject(emptyMap()),
    val children: List<HomeItem> = emptyList(),
) {
    val isApp: Boolean get() = kind == Kinds.APP
    val isShortcut: Boolean get() = kind == Kinds.SHORTCUT
    val isFolder: Boolean get() = kind == Kinds.FOLDER

    /** App or shortcut: what may go into a folder. */
    val canBeInFolder: Boolean get() = isApp || isShortcut

    fun attr(name: String): String? = (attrs[name] as? JsonPrimitive)?.contentOrNull

    fun attrLong(name: String): Long? = (attrs[name] as? JsonPrimitive)?.longOrNull

    fun withAttr(name: String, value: JsonElement): HomeItem = copy(attrs = JsonObject(attrs + (name to value)))

    /** `component` of an app or shortcut as `<package>/<class>`. */
    val component: String? get() = attr("component")

    /** Package of [component]; also valid for a shortcut, whose activity may be unknown (`<package>/`). */
    val packageName: String? get() = component?.substringBefore('/')?.takeIf { it.isNotEmpty() }

    /** Serial number of the profile the app belongs to. */
    val userSerial: Long get() = attrLong("user") ?: 0L

    /** Name of a folder. */
    val name: String get() = attr("name") ?: FOLDER_NAME

    companion object {
        const val FOLDER_NAME = "Ordner"
    }
}

/** Informative grid values and the icon size; columns and rows are recomputed on every display (U35-20, U35-21). */
data class GridSpec(val columns: Int, val rows: Int, val iconDp: Int)

/**
 * The file `layout.json` (schema 1): pages of items, the dock, and [extra] with every unknown top-level
 * key (`seSeeded`, `showPrivateSpace`, later ones), kept as read.
 */
data class HomeLayout(
    val grid: GridSpec,
    val pages: List<List<HomeItem>>,
    val dock: List<HomeItem>,
    val extra: JsonObject = JsonObject(emptyMap()),
)

/** Result of reading `layout.json`. */
sealed interface LayoutReadResult {
    data class Loaded(val layout: HomeLayout) : LayoutReadResult

    data object Missing : LayoutReadResult

    /** [reason] is for the log; the file is set aside as `layout.json.corrupt`. */
    data class Corrupt(val reason: String) : LayoutReadResult
}
