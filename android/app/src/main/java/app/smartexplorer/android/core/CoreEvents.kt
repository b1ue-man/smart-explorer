package app.smartexplorer.android.core

import android.util.Log
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
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
                "screenView" -> decodeScreenView(event)
                "vault" -> event.text("site")?.let { VaultChanged(it, event.text("state").orEmpty(), event.text("reason")) }
                "docs.changed" -> DocsChanged(
                    roots = (event["roots"] as? JsonPrimitive)?.booleanOrNull ?: false,
                    documents = (event["documents"] as? JsonArray)
                        ?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull }
                        .orEmpty(),
                    commitFailed = (event["commitFailed"] as? JsonPrimitive)?.booleanOrNull ?: false,
                )
                else -> null
            }
        } catch (e: IllegalArgumentException) {
            // SerializationException is an IllegalArgumentException: skip only this event.
            Log.w(TAG, "skipping malformed event", e)
            null
        }
    }

    /** The `screenView` event of a viewer session (SCR-M9); `null` for an unknown kind. */
    internal fun decodeScreenView(event: JsonObject): ScreenViewEvent? {
        val session = (event["session"] as? JsonPrimitive)?.longOrNull ?: return null
        val kind = event.text("kind") ?: return null
        return when (kind) {
            "pending" -> ScreenViewEvent.Pending(session, event.text("capability").orEmpty())
            "accepted" -> ScreenViewEvent.Accepted(
                session = session,
                path = event.text("path").orEmpty(),
                platform = event.text("platform").orEmpty(),
                view = (event["authorization"] as? JsonObject)?.text("view") ?: "off",
                control = (event["authorization"] as? JsonObject)?.text("control") ?: "off",
                body = event,
            )
            "state" -> ScreenViewEvent.State(session, event.text("state").orEmpty(), event.text("reason"))
            "stream" -> ScreenViewEvent.Stream(
                session = session,
                codec = event.text("codec").orEmpty(),
                profile = event.text("profile"),
                width = (event["width"] as? JsonPrimitive)?.intOrNull ?: 0,
                height = (event["height"] as? JsonPrimitive)?.intOrNull ?: 0,
                gop = (event["gop"] as? JsonPrimitive)?.longOrNull ?: 0L,
                encoder = event.text("encoder").orEmpty(),
                hardware = (event["hardware"] as? JsonPrimitive)?.booleanOrNull ?: false,
            )
            "stats" -> ScreenViewEvent.Stats(session, event)
            "controlState" -> ScreenViewEvent.ControlState(session, event.text("holder"), event.text("blocked"))
            "windows" -> ScreenViewEvent.Windows(session, event)
            "error" -> ScreenViewEvent.Error(session, event.text("code").orEmpty(), event.text("detail"))
            "ended" -> ScreenViewEvent.Ended(session, event.text("code").orEmpty(), event.text("detail"))
            else -> null
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

/**
 * A vault site changed (VLT-M11, U31-17): [state] `locked`, `unlocked` or `locking`; [reason] for a
 * lock `screen_off`, `background`, `idle`, `manual`, `process_end` or `null`.
 */
data class VaultChanged(val site: String, val state: String, val reason: String?) : CoreEvent

/**
 * Documents provider (DRV-M8): [roots] came or went, the listings of [documents] changed, or a
 * change other apps saved could not be uploaded ([commitFailed]; it stays queued).
 */
data class DocsChanged(val roots: Boolean, val documents: List<String>, val commitFailed: Boolean) : CoreEvent

/**
 * Progress of a viewer session of this phone (SCR-M9, `screen.viewOpen`): `type` `screenView` with
 * a `kind`. [Accepted], [Stats] and [Windows] keep the whole event as [body] (the structured parts
 * are decoded by the screen code); the other kinds carry their fields.
 */
sealed interface ScreenViewEvent : CoreEvent {
    val session: Long

    /** The device asks its user; [capability] is `screenView` or `screenControl`. */
    data class Pending(override val session: Long, val capability: String) : ScreenViewEvent

    /** The device accepted: [path] `direct` or `relay`, [view]/[control] `off`, `ask` or `allow`. */
    data class Accepted(
        override val session: Long,
        val path: String,
        val platform: String,
        val view: String,
        val control: String,
        val body: JsonObject,
    ) : ScreenViewEvent

    /** A `SessionState` key with its reason key. */
    data class State(override val session: Long, val state: String, val reason: String?) : ScreenViewEvent

    /** A new picture group starts; [gop] is the number whose low 16 bits mark its pictures. */
    data class Stream(
        override val session: Long,
        val codec: String,
        val profile: String?,
        val width: Int,
        val height: Int,
        val gop: Long,
        val encoder: String,
        val hardware: Boolean,
    ) : ScreenViewEvent

    data class Stats(override val session: Long, val body: JsonObject) : ScreenViewEvent

    /** [holder] `viewer`, `other` or `null`; [blocked] a `ControlBlock` key. */
    data class ControlState(override val session: Long, val holder: String?, val blocked: String?) : ScreenViewEvent

    data class Windows(override val session: Long, val body: JsonObject) : ScreenViewEvent

    data class Error(override val session: Long, val code: String, val detail: String?) : ScreenViewEvent

    /** The session ended; [code] is an `EndReason` or error code key. */
    data class Ended(override val session: Long, val code: String, val detail: String?) : ScreenViewEvent
}
