package app.smartexplorer.android.launcher.model

import java.util.UUID
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.put

/**
 * Reads and writes `layout.json` (U35-18), pure Kotlin. Schema 1:
 * `{"schema":1,"grid":{"columns","rows","iconDp"},"pages":[{"items":[{"id","kind","cell":[x,y],"span":[w,h],…}]}],"dock":[…],…}`.
 * Keys this version does not know stay in [HomeItem.attrs] or [HomeLayout.extra]; unknown kinds stay as items.
 */
object LayoutJson {
    private const val SCHEMA = 1
    private val RESERVED = setOf("id", "kind", "cell", "span", "children")
    private val TOP_LEVEL = setOf("schema", "grid", "pages", "dock")

    const val DEFAULT_COLUMNS = 4
    const val DEFAULT_ROWS = 5
    const val DEFAULT_ICON_DP = 56

    /** `schema` missing or not 1, or invalid JSON → [LayoutReadResult.Corrupt]. */
    fun parse(text: String): LayoutReadResult {
        val root = try {
            Json.parseToJsonElement(text) as? JsonObject
        } catch (e: IllegalArgumentException) {
            return LayoutReadResult.Corrupt("kein gültiges JSON: ${e.message}")
        } ?: return LayoutReadResult.Corrupt("kein JSON-Objekt")
        val schema = (root["schema"] as? JsonPrimitive)?.intOrNull
        if (schema != SCHEMA) return LayoutReadResult.Corrupt("schema ist $schema, erwartet $SCHEMA")

        val ids = IdRegistry()
        val grid = root["grid"] as? JsonObject
        val spec = GridSpec(
            columns = grid.intOr("columns", DEFAULT_COLUMNS).coerceAtLeast(1),
            rows = grid.intOr("rows", DEFAULT_ROWS).coerceAtLeast(1),
            iconDp = grid.intOr("iconDp", DEFAULT_ICON_DP),
        )
        val pages = (root["pages"] as? JsonArray).orEmpty().map { page ->
            val items = when (page) {
                is JsonObject -> page["items"] as? JsonArray
                is JsonArray -> page
                else -> null
            }
            items.orEmpty().mapNotNull { (it as? JsonObject)?.let { entry -> parseItem(entry, withCell = true, ids = ids) } }
        }
        val dock = (root["dock"] as? JsonArray).orEmpty().mapNotNull { (it as? JsonObject)?.let { entry -> parseItem(entry, true, ids) } }
        val extra = JsonObject(root.filterKeys { it !in TOP_LEVEL })
        return LayoutReadResult.Loaded(HomeLayout(spec, pages.ifEmpty { listOf(emptyList()) }, dock, extra))
    }

    fun encode(layout: HomeLayout): String = buildJsonObject {
        put("schema", SCHEMA)
        put(
            "grid",
            buildJsonObject {
                put("columns", layout.grid.columns)
                put("rows", layout.grid.rows)
                put("iconDp", layout.grid.iconDp)
            },
        )
        put(
            "pages",
            JsonArray(
                layout.pages.map { page ->
                    buildJsonObject { put("items", JsonArray(page.map { encodeItem(it, child = false) })) }
                },
            ),
        )
        put("dock", JsonArray(layout.dock.map { encodeItem(it, child = false) }))
        layout.extra.entries.sortedBy { it.key }.forEach { (key, value) -> put(key, value) }
    }.toString()

    /** A fresh item id: `i` and the first 12 hex characters of a random UUID; re-rolled on a collision. */
    fun newId(taken: Set<String>): String {
        while (true) {
            val id = "i" + UUID.randomUUID().toString().replace("-", "").take(12)
            if (id !in taken) return id
        }
    }

    private class IdRegistry {
        private val seen = HashSet<String>()

        /** The second occurrence of an id becomes `<id>-2`, then `-3` … (U35-18). */
        fun unique(raw: String?): String {
            val base = raw?.takeIf { it.isNotBlank() } ?: newId(seen)
            if (seen.add(base)) return base
            var n = 2
            while (!seen.add("$base-$n")) n++
            return "$base-$n"
        }
    }

    private fun parseItem(entry: JsonObject, withCell: Boolean, ids: IdRegistry): HomeItem {
        val id = ids.unique((entry["id"] as? JsonPrimitive)?.contentOrNull)
        val kind = (entry["kind"] as? JsonPrimitive)?.contentOrNull.orEmpty()
        val cell = if (withCell) pair(entry["cell"])?.let { Cell(it.first, it.second) } ?: Cell(0, 0) else Cell(0, 0)
        val span = if (withCell) {
            pair(entry["span"])?.let { Span(it.first.coerceAtLeast(1), it.second.coerceAtLeast(1)) } ?: Span(1, 1)
        } else {
            Span(1, 1)
        }
        val children = (entry["children"] as? JsonArray).orEmpty()
            .mapNotNull { (it as? JsonObject)?.let { child -> parseItem(child, withCell = false, ids = ids) } }
        return HomeItem(id, kind, cell, span, JsonObject(entry.filterKeys { it !in RESERVED }), children)
    }

    private fun encodeItem(item: HomeItem, child: Boolean): JsonObject = buildJsonObject {
        put("id", item.id)
        put("kind", item.kind)
        if (!child) {
            put("cell", JsonArray(listOf(JsonPrimitive(item.cell.x), JsonPrimitive(item.cell.y))))
            put("span", JsonArray(listOf(JsonPrimitive(item.span.w), JsonPrimitive(item.span.h))))
        }
        item.attrs.forEach { (key, value) -> put(key, value) }
        if (item.isFolder || item.children.isNotEmpty()) {
            put("children", JsonArray(item.children.map { encodeItem(it, child = true) }))
        }
    }

    private fun pair(element: JsonElement?): Pair<Int, Int>? {
        val array = element as? JsonArray ?: return null
        if (array.size < 2) return null
        val first = (array[0] as? JsonPrimitive)?.intOrNull ?: return null
        val second = (array[1] as? JsonPrimitive)?.intOrNull ?: return null
        return first to second
    }

    private fun JsonObject?.intOr(name: String, fallback: Int): Int = (this?.get(name) as? JsonPrimitive)?.intOrNull ?: fallback
}
