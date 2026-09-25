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
                "edits" -> CoreEvent.Edits
                "openUrl" -> event.text("url")?.let { CoreEvent.OpenUrl(it) }
                "error" -> CoreEvent.Error(event.text("action").orEmpty(), event.text("message").orEmpty())
                "volumes" -> CoreEvent.Volumes
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
