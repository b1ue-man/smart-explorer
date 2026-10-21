package app.smartexplorer.android.launcher.model

import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull

/** Cell of the grid, counted from the top left (U35-18). */
data class Cell(val x: Int, val y: Int)

/** Width and height in cells. */
data class Span(val w: Int, val h: Int)

/**
 * Kinds this package draws itself; U38 added `widget`, U40 added the four se kinds (U35-31, U40-9): a device, a room, a
 * vault site (each with `attrs.target`, the link to the entry of `DEVICES`) and „Alle Bildschirme“. [SE_ADD_DEVICE]
 * and [SE_JOIN_ROOM] are the two tiles of an empty device list; they are computed at drawing time and never stored.
 */
object Kinds {
    const val APP = "app"
    const val SHORTCUT = "shortcut"
    const val FOLDER = "folder"
    const val WIDGET = "widget"
    const val SE_DEVICE = "seDevice"
    const val SE_ROOM = "seRoom"
    const val SE_VAULT = "seVault"
    const val SE_SCREENS = "seScreens"
    const val SE_ADD_DEVICE = "seAddDevice"
    const val SE_JOIN_ROOM = "seJoinRoom"

    /** The four stored se kinds. */
    val SE_STORED: Set<String> = setOf(SE_DEVICE, SE_ROOM, SE_VAULT, SE_SCREENS)
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
    val isWidget: Boolean get() = kind == Kinds.WIDGET

    /** One of the four stored se kinds (device, room, vault, all screens). */
    val isSe: Boolean get() = kind in Kinds.SE_STORED

    /** App or shortcut: what may go into a folder. */
    val canBeInFolder: Boolean get() = isApp || isShortcut

    fun attr(name: String): String? = (attrs[name] as? JsonPrimitive)?.contentOrNull

    fun attrLong(name: String): Long? = (attrs[name] as? JsonPrimitive)?.longOrNull

    fun attrBool(name: String): Boolean? = (attrs[name] as? JsonPrimitive)?.booleanOrNull

    fun withAttr(name: String, value: JsonElement): HomeItem = copy(attrs = JsonObject(attrs + (name to value)))

    /** `component` of an app or shortcut as `<package>/<class>`. */
    val component: String? get() = attr("component")

    /** Package of [component]; also valid for a shortcut, whose activity may be unknown (`<package>/`). */
    val packageName: String? get() = component?.substringBefore('/')?.takeIf { it.isNotEmpty() }

    /** Serial number of the profile the app belongs to. */
    val userSerial: Long get() = attrLong("user") ?: 0L

    /** Widget (U38-6): `<package>/<class>` of the provider and the id the widget host gave it (`null` when unknown). */
    val provider: String? get() = attr("provider")

    val providerPackage: String? get() = provider?.substringBefore('/')?.takeIf { it.isNotEmpty() }

    val widgetId: Int? get() = attrLong("widgetId")?.toInt()

    /** False while a widget still has to be set up by its configuration activity (U38-6); missing means set up. */
    val configured: Boolean get() = attrBool("configured") ?: true

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
