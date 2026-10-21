package app.smartexplorer.android.widget

import android.content.Context
import android.util.AtomicFile
import app.smartexplorer.android.launcher.bridge.BridgeProtocol
import app.smartexplorer.android.launcher.bridge.DevicesData
import app.smartexplorer.android.launcher.bridge.StateTexts
import app.smartexplorer.android.launcher.bridge.StatusCodes
import java.io.File
import java.io.IOException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.put

/** What a tap on a row of the widget opens (U40-16): the device's folder, or the Share tab (access not yet approved, identity changed). */
sealed interface WidgetTarget {
    data class OpenLocation(val location: String) : WidgetTarget

    data class SelectTab(val tab: String) : WidgetTarget

    /** The `NAV_KIND` and `NAV_VALUE` of the fill-in intent, in the wire form of `AppNav`. */
    fun wire(): Pair<String, String> = when (this) {
        is OpenLocation -> "openLocation" to location.trimEnd('/') + "/"
        is SelectTab -> "selectTab" to tab
    }
}

/** One row: the device name (a room device as „Raum · Gerät“), its state text from the table of U40-9, and whether it is online. */
data class WidgetRow(val name: String, val state: String, val online: Boolean, val target: WidgetTarget)

/** The widget cache: the rows and the navigation token, which this app-private file may hold (U40-16). */
data class WidgetSnapshot(val rows: List<WidgetRow>, val token: String?)

/**
 * `<filesDir>/widget/devices-cache.json` (U40-16): `{"token":"…","rows":[{"name","state","online","target":{"kind","value"}}]}`.
 * Written by the feed in the main process, read by the provider and the list service in the launcher process. It lives in
 * `widget`, which imports nothing from the main process but `launcher.bridge`.
 */
object WidgetCache {
    private val json = Json { ignoreUnknownKeys = true }

    fun file(context: Context): File = File(File(context.filesDir, BridgeProtocol.WIDGET_DIR), BridgeProtocol.WIDGET_CACHE_FILE)

    /** The rows of [data]: every direct device in list order, then every room device. */
    fun rowsOf(data: DevicesData): List<WidgetRow> {
        fun target(code: String, location: String): WidgetTarget =
            if (code == StatusCodes.WAITING_FOR_ACCESS || code == StatusCodes.IDENTITY_CONFLICT) WidgetTarget.SelectTab("Share") else WidgetTarget.OpenLocation(location)
        val direct = data.devices.map { WidgetRow(it.name, StateTexts.text(it.status, it.lan), it.online, target(it.status, it.location)) }
        val rooms = data.rooms.flatMap { room ->
            room.members.map { WidgetRow("${room.name} · ${it.name}", StateTexts.text(it.status), it.online, target(it.status, it.location)) }
        }
        return direct + rooms
    }

    fun encode(snapshot: WidgetSnapshot): String = buildJsonObject {
        put("token", snapshot.token)
        put(
            "rows",
            buildJsonArray {
                snapshot.rows.forEach { row ->
                    val (kind, value) = row.target.wire()
                    add(
                        buildJsonObject {
                            put("name", row.name)
                            put("state", row.state)
                            put("online", row.online)
                            put("target", buildJsonObject { put("kind", kind); put("value", value) })
                        },
                    )
                }
            },
        )
    }.toString()

    /** `null` for text that is no object. A row with a target this version does not know is dropped. */
    fun parse(text: String): WidgetSnapshot? {
        val root = try {
            json.parseToJsonElement(text) as? JsonObject
        } catch (_: IllegalArgumentException) {
            null
        } ?: return null
        val rows = (root["rows"] as? JsonArray).orEmpty().mapNotNull { element ->
            val row = element as? JsonObject ?: return@mapNotNull null
            val target = row["target"] as? JsonObject ?: return@mapNotNull null
            val kind = (target["kind"] as? JsonPrimitive)?.contentOrNull
            val value = (target["value"] as? JsonPrimitive)?.contentOrNull ?: return@mapNotNull null
            val parsed = when (kind) {
                "openLocation" -> WidgetTarget.OpenLocation(value)
                "selectTab" -> WidgetTarget.SelectTab(value)
                else -> return@mapNotNull null
            }
            WidgetRow(
                name = (row["name"] as? JsonPrimitive)?.contentOrNull.orEmpty(),
                state = (row["state"] as? JsonPrimitive)?.contentOrNull.orEmpty(),
                online = (row["online"] as? JsonPrimitive)?.booleanOrNull ?: false,
                target = parsed,
            )
        }
        val token = (root["token"] as? JsonPrimitive)?.takeIf { it !is JsonNull }?.contentOrNull
        return WidgetSnapshot(rows, token)
    }

    /** The stored snapshot; `null` when the file is missing or unreadable („Noch keine Daten.“). */
    fun read(context: Context): WidgetSnapshot? {
        val file = file(context)
        if (!file.isFile) return null
        return try {
            parse(file.readText(Charsets.UTF_8))
        } catch (_: IOException) {
            null
        }
    }

    @Throws(IOException::class)
    fun write(context: Context, snapshot: WidgetSnapshot) {
        val file = file(context)
        val dir = file.parentFile ?: throw IOException("Ordner fehlt: ${file.path}")
        if (!dir.isDirectory && !dir.mkdirs() && !dir.isDirectory) throw IOException("Ordner nicht anlegbar: ${dir.path}")
        val atomic = AtomicFile(file)
        val out = atomic.startWrite()
        try {
            out.write(encode(snapshot).toByteArray(Charsets.UTF_8))
            atomic.finishWrite(out)
        } catch (e: IOException) {
            atomic.failWrite(out)
            throw e
        }
    }
}
