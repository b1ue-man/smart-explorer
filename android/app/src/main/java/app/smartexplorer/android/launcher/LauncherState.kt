package app.smartexplorer.android.launcher

import android.content.Context
import android.util.AtomicFile
import java.io.File
import java.io.IOException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject

/** Which parts of the launcher module are switched on (U35-4). */
data class Parts(val launcher: Boolean, val widget: Boolean)

/**
 * Content of `<filesDir>/launcher/state.json` (U35-4): `{"schema":1,"state":"enabled|disabled",
 * "parts":{"launcher":bool,"widget":bool},"previousHome":"<package>/<class>|null","changedAt":<ms>,
 * "noticeDismissed":bool}`. The launcher component itself is the source of truth for „enabled“
 * (U35-5); this file holds the rest.
 */
data class LauncherStateData(
    val state: String,
    val parts: Parts,
    val previousHome: String?,
    val changedAt: Long,
    val noticeDismissed: Boolean,
) {
    val enabled: Boolean get() = state == STATE_ENABLED

    companion object {
        const val STATE_ENABLED = "enabled"
        const val STATE_DISABLED = "disabled"

        /** What a missing, unreadable or foreign-schema file means (U35-4). */
        val DISABLED = LauncherStateData(STATE_DISABLED, Parts(launcher = false, widget = false), null, 0L, false)
    }
}

/**
 * Reads and writes `state.json` (U35-4). Written only by the main process, always through
 * [AtomicFile]; the `:launcher` process may read it but never needs to. [parse] and [encode] are
 * pure so the JVM tests can reach them.
 */
object LauncherState {
    private const val SCHEMA = 1
    private val json = Json { ignoreUnknownKeys = true }

    /** `<filesDir>/launcher/`, owned by the `:launcher` process except `state.json` (U35-15). */
    fun dir(context: Context): File = File(context.filesDir, "launcher")

    fun file(context: Context): File = File(dir(context), "state.json")

    /**
     * Empty marker the launcher creates when it set a corrupt `layout.json` aside (U35-18); the card shows
     * a notice while it exists and deletes it on „Verstanden“.
     */
    fun recoveredFile(context: Context): File = File(dir(context), "recovered")

    /** The stored state, or [LauncherStateData.DISABLED] when the file is missing or unusable (it is not deleted). */
    fun read(context: Context): LauncherStateData {
        val file = file(context)
        if (!file.isFile) return LauncherStateData.DISABLED
        return try {
            parse(file.readText()) ?: LauncherStateData.DISABLED
        } catch (_: IOException) {
            LauncherStateData.DISABLED
        }
    }

    /** Replaces the file atomically (`startWrite`/`finishWrite`/`failWrite`). */
    @Throws(IOException::class)
    fun write(context: Context, data: LauncherStateData) {
        val dir = dir(context)
        if (!dir.isDirectory && !dir.mkdirs() && !dir.isDirectory) throw IOException("Ordner nicht anlegbar: ${dir.path}")
        val atomic = AtomicFile(file(context))
        val out = atomic.startWrite()
        try {
            out.write(encode(data).toByteArray(Charsets.UTF_8))
            atomic.finishWrite(out)
        } catch (e: IOException) {
            atomic.failWrite(out)
            throw e
        }
    }

    /** `null` for invalid JSON or a `schema` other than 1. Unknown keys are ignored. */
    internal fun parse(text: String): LauncherStateData? {
        val root = try {
            json.parseToJsonElement(text) as? JsonObject
        } catch (_: IllegalArgumentException) {
            null
        } ?: return null
        if ((root["schema"] as? JsonPrimitive)?.intOrNull != SCHEMA) return null
        val state = (root["state"] as? JsonPrimitive)?.contentOrNull
        if (state != LauncherStateData.STATE_ENABLED && state != LauncherStateData.STATE_DISABLED) return null
        val parts = root["parts"] as? JsonObject
        val previous = (root["previousHome"] as? JsonPrimitive)?.takeIf { it !is JsonNull }?.contentOrNull
        return LauncherStateData(
            state = state,
            parts = Parts(
                launcher = (parts?.get("launcher") as? JsonPrimitive)?.booleanOrNull ?: false,
                widget = (parts?.get("widget") as? JsonPrimitive)?.booleanOrNull ?: false,
            ),
            previousHome = previous,
            changedAt = (root["changedAt"] as? JsonPrimitive)?.longOrNull ?: 0L,
            noticeDismissed = (root["noticeDismissed"] as? JsonPrimitive)?.booleanOrNull ?: false,
        )
    }

    internal fun encode(data: LauncherStateData): String = buildJsonObject {
        put("schema", SCHEMA)
        put("state", data.state)
        putJsonObject("parts") {
            put("launcher", data.parts.launcher)
            put("widget", data.parts.widget)
        }
        put("previousHome", data.previousHome)
        put("changedAt", data.changedAt)
        put("noticeDismissed", data.noticeDismissed)
    }.toString()
}
