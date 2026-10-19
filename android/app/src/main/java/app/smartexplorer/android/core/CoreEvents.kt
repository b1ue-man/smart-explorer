package app.smartexplorer.android.core

import android.util.Log
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull

/** Decoding of the `pollEvents` envelope (api.md §3). */
internal object CoreEvents {
    private const val TAG = "SmartExplorerCore"

    /** Events of one poll, or `null` when the envelope is an error or unreadable. */
    fun parse(json: Json, raw: String): List<CoreEvent>? {
        val envelope = try {
            json.parseToJsonElement(raw) as? JsonObject
        } catch (e: SerializationException) {
            Log.w(TAG, "unreadable event envelope", e)
            null
        } ?: return null
        val events = envelope["ok"] as? JsonArray
        if (events == null) {
            Log.w(TAG, "event poll failed: ${envelope["err"]}")
            return null
        }
        return events.mapNotNull { decode(json, it) }
    }

    private fun decode(json: Json, element: JsonElement): CoreEvent? {
        val event = element as? JsonObject ?: return null
        return try {
            when (event.text("type")) {
                "task" -> event["task"]?.let { CoreEvent.Task(json.decodeFromJsonElement(TaskInfo.serializer(), it)) }
                "share" -> CoreEvent.Share
                "shareRequest" -> CoreEvent.ShareRequest((event["count"] as? JsonPrimitive)?.intOrNull ?: 1)
                "jobs" -> CoreEvent.Jobs
                "syncProblem" -> event.text("jobId")?.let { CoreEvent.SyncProblem(it, event.text("title").orEmpty(), event.text("text").orEmpty()) }
                "edits" -> CoreEvent.Edits
                "openUrl" -> event.text("url")?.let { CoreEvent.OpenUrl(it) }
                "error" -> CoreEvent.Error(event.text("action").orEmpty(), event.text("message").orEmpty())
                "volumes" -> CoreEvent.Volumes
                "wake" -> (event["ms"] as? JsonPrimitive)?.longOrNull?.takeIf { it > 0 }?.let { CoreEvent.Wake(it) }
                "screenHostStart" -> (event["session"] as? JsonPrimitive)?.longOrNull?.let { session ->
                    ScreenHostStart(
                        session = session,
                        source = event.text("source").orEmpty(),
                        still = (event["still"] as? JsonPrimitive)?.contentOrNull == "true",
                        maxWidth = (event["maxWidth"] as? JsonPrimitive)?.intOrNull ?: 0,
                        maxHeight = (event["maxHeight"] as? JsonPrimitive)?.intOrNull ?: 0,
                        viewerName = event.text("viewerName").orEmpty(),
                        viewerDeviceId = event.text("viewerDeviceId").orEmpty(),
                    )
                }
                "screenSession" -> (event["session"] as? JsonPrimitive)?.longOrNull?.let { session ->
                    ScreenSessionChanged(session, event.text("state").orEmpty(), event.text("reason"))
                }
                else -> null
            }
        } catch (e: IllegalArgumentException) {
            // SerializationException is an IllegalArgumentException: skip only this event.
            Log.w(TAG, "skipping malformed event", e)
            null
        }
    }

    private fun JsonObject.text(key: String): String? = (this[key] as? JsonPrimitive)?.contentOrNull
}

/**
 * The core starts a screen session of this phone as host (SCR-M8): the app asks its user (system
 * dialog „Bildschirm teilen“, except for [still] previews and the test picture `synthetic`) and
 * then captures [source] for the viewer [viewerName].
 */
data class ScreenHostStart(
    val session: Long,
    val source: String,
    val still: Boolean,
    val maxWidth: Int,
    val maxHeight: Int,
    val viewerName: String,
    val viewerDeviceId: String,
) : CoreEvent

/** A host session changed: `consent`, `streaming` or `ended` (with its [reason]). */
data class ScreenSessionChanged(val session: Long, val state: String, val reason: String?) : CoreEvent
