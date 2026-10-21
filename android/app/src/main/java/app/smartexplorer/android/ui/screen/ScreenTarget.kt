package app.smartexplorer.android.ui.screen

import app.smartexplorer.android.api.ScreenDevice
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.put

/**
 * The screen of one device (SCR-M9, contract V5): [relation] `direct` (a contact, [relationId] its
 * ID) or `room` (a member of the room with the local profile ID [relationId]; [deviceId] is then
 * required). A `null` [source] is the whole (primary) screen; [control] starts with control.
 */
data class ScreenTarget(
    val relation: String,
    val relationId: String,
    val deviceId: String?,
    val source: String? = null,
    val control: Boolean = false,
) {
    /** The JSON of `NavRequest.OpenScreen`: `{"relation","relation_id","device_id","source","control"}`. */
    fun toJson(): String = ScreenTargetJson.encode(this)

    /** Same device, whatever source and control are. */
    fun sameDevice(other: ScreenTarget): Boolean =
        relation == other.relation && relationId == other.relationId && deviceId == other.deviceId
}

/** The target of a device of `screen.devices`. */
fun ScreenDevice.toTarget(control: Boolean = false, source: String? = null) =
    ScreenTarget(relation, relationId, deviceId.ifBlank { null }, source, control)

/** Reads and writes the target JSON of contract V5. */
object ScreenTargetJson {
    private val json = Json { ignoreUnknownKeys = true }

    /** The target in [text], or a failure whose message is the German error text. */
    fun parse(text: String): Result<ScreenTarget> = runCatching {
        val obj = try {
            json.parseToJsonElement(text) as? JsonObject
        } catch (e: IllegalArgumentException) {
            null
        } ?: throw IllegalArgumentException("Ungültiges Bildschirmziel")
        fun field(key: String) = (obj[key] as? JsonPrimitive)?.takeIf { it !is JsonNull }?.contentOrNull
        val relation = field("relation")?.takeIf { it == "direct" || it == "room" }
            ?: throw IllegalArgumentException("Ungültiges Bildschirmziel: relation ist direct oder room")
        val relationId = field("relation_id")?.trim().orEmpty()
        if (relationId.isEmpty()) throw IllegalArgumentException("Ungültiges Bildschirmziel: relation_id fehlt")
        val deviceId = field("device_id")?.takeIf { it.isNotEmpty() }
        if (relation == "room" && deviceId == null) throw IllegalArgumentException("Das Raumgerät fehlt im Bildschirmziel")
        ScreenTarget(
            relation = relation,
            relationId = relationId,
            deviceId = deviceId,
            source = field("source")?.takeIf { it.isNotEmpty() },
            control = (obj["control"] as? JsonPrimitive)?.booleanOrNull ?: false,
        )
    }

    fun encode(target: ScreenTarget): String = buildJsonObject {
        put("relation", target.relation)
        put("relation_id", target.relationId)
        put("device_id", target.deviceId)
        put("source", target.source)
        put("control", target.control)
    }.toString()
}
