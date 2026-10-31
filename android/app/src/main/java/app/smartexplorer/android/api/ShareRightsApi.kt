package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonObjectBuilder
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// Rights of a new connection (plan 2026-10-31 §F, F1–F7; api.md §5.1): presets, open confirmations
// („Rechte offen“), „Bestätigen“, „Abbrechen“ and the rights an Direct code / PIN offer / room gives.
// Field names exactly as the contract.

/** `Rights.folders.kind`: no folder, every folder of the scope, or exactly [RightsFolders.paths]. */
const val FOLDERS_NONE = "none"
const val FOLDERS_ALL = "all"
const val FOLDERS_SELECTED = "selected"

/** `RightsTarget.kind`. */
const val RIGHTS_TARGET_DIRECT = "direct"
const val RIGHTS_TARGET_ROOM = "room"

/** The only account commands run as on Android. */
const val RIGHTS_ACCOUNT_OWN = "own"

/** Keys of the presets (the same on every surface). */
const val PRESET_NONE = "none"
const val PRESET_READ_ONLY = "read-only"
const val PRESET_READ_WRITE = "read-write"
const val PRESET_FILES_EXEC = "files-exec"
const val PRESET_ALL = "all"

/** The fields of a [Rights] a call can name (`rights` of `share.confirmRights` and the others). */
const val RIGHTS_FIELD_FOLDERS = "folders"
const val RIGHTS_FIELD_WRITE = "write"
const val RIGHTS_FIELD_COMMANDS = "commands"
const val RIGHTS_FIELD_TERMINAL = "terminal"
const val RIGHTS_FIELD_SCREEN_VIEW = "screenView"
const val RIGHTS_FIELD_SCREEN_CONTROL = "screenControl"
const val RIGHTS_FIELD_VALID_UNTIL = "validUntil"
const val RIGHTS_FIELD_ACCOUNT = "account"

/** Every field in the order the sheet shows them. */
val RIGHTS_FIELDS: List<String> = listOf(
    RIGHTS_FIELD_FOLDERS,
    RIGHTS_FIELD_WRITE,
    RIGHTS_FIELD_COMMANDS,
    RIGHTS_FIELD_TERMINAL,
    RIGHTS_FIELD_SCREEN_VIEW,
    RIGHTS_FIELD_SCREEN_CONTROL,
    RIGHTS_FIELD_VALID_UNTIL,
    RIGHTS_FIELD_ACCOUNT,
)

/** `Rights.folders`; [connections] only counts for [FOLDERS_SELECTED]. */
@Serializable
data class RightsFolders(
    val kind: String = FOLDERS_NONE,
    val paths: List<String> = emptyList(),
    val connections: Boolean? = null,
)

/**
 * What a connection may do on this phone. [preset] and [summary] come only in answers; a decision
 * is `off|ask|allow`, [validUntil] is Unix seconds.
 */
@Serializable
data class Rights(
    val folders: RightsFolders = RightsFolders(),
    val write: Boolean = false,
    val commands: String = ACCESS_OFF,
    val terminal: String = ACCESS_OFF,
    val screenView: String = ACCESS_OFF,
    val screenControl: String = ACCESS_OFF,
    val validUntil: Long? = null,
    val account: String = RIGHTS_ACCOUNT_OWN,
    val preset: String? = null,
    val summary: String = "",
) {
    /** The decision of the capability [field] (`commands`, `terminal`, `screenView`, `screenControl`). */
    fun decision(field: String): String = when (field) {
        RIGHTS_FIELD_COMMANDS -> commands
        RIGHTS_FIELD_TERMINAL -> terminal
        RIGHTS_FIELD_SCREEN_VIEW -> screenView
        RIGHTS_FIELD_SCREEN_CONTROL -> screenControl
        else -> ACCESS_OFF
    }

    fun withDecision(field: String, decision: String): Rights = when (field) {
        RIGHTS_FIELD_COMMANDS -> copy(commands = decision)
        RIGHTS_FIELD_TERMINAL -> copy(terminal = decision)
        RIGHTS_FIELD_SCREEN_VIEW -> copy(screenView = decision)
        RIGHTS_FIELD_SCREEN_CONTROL -> copy(screenControl = decision)
        else -> this
    }
}

/** One connection the rights belong to: a Direct device (exact pins) or a room (its profile id). */
@Serializable
data class RightsTarget(
    val kind: String,
    val deviceId: String? = null,
    val publicKey: String? = null,
    val profileId: String? = null,
) {
    /** Stable key of the target, for sets and maps. */
    val key: String
        get() = if (kind == RIGHTS_TARGET_ROOM) "room/${profileId.orEmpty()}" else "direct/${deviceId.orEmpty()}/${publicKey.orEmpty()}"

    val isRoom: Boolean
        get() = kind == RIGHTS_TARGET_ROOM

    fun toJson(): JsonObject = buildJsonObject {
        put("kind", kind)
        if (isRoom) {
            put("profileId", profileId.orEmpty())
        } else {
            put("deviceId", deviceId.orEmpty())
            put("publicKey", publicKey.orEmpty())
        }
    }

    companion object {
        fun direct(deviceId: String, publicKey: String) = RightsTarget(RIGHTS_TARGET_DIRECT, deviceId = deviceId, publicKey = publicKey)

        fun room(profileId: String) = RightsTarget(RIGHTS_TARGET_ROOM, profileId = profileId)
    }
}

/**
 * A connection that waits for its rights („Rechte offen“); [origin] is `request|pairing|share_back|
 * repair|identity_replaced|readmitted|room_created|room_joined`, [since] Unix seconds, [rights] what
 * the confirmation starts with.
 */
@Serializable
data class PendingRights(
    val target: RightsTarget,
    val origin: String = "",
    val since: Long = 0,
    val name: String = "",
    val rights: Rights = Rights(),
)

/** One preset of `share.rightsPresets`. */
@Serializable
data class RightsPresetInfo(
    val key: String,
    val label: String = "",
    val description: String = "",
    val rights: Rights = Rights(),
)

/** `share.pendingRights`. */
@Serializable
data class PendingRightsAnswer(
    val pendingRights: List<PendingRights> = emptyList(),
    val codeRights: Rights? = null,
)

/** The answer of `share.confirmRights`, `share.cancelRights` and `share.setCodeRights`. */
@Serializable
data class RightsChange(
    val changed: Boolean = false,
    val persisted: Boolean = false,
    val pendingRights: List<PendingRights> = emptyList(),
    val codeRights: Rights? = null,
)

@Serializable
private data class PresetsAnswer(val presets: List<RightsPresetInfo> = emptyList())

/**
 * The rights a call names: a [preset] key (fills every field), changed by the [rights] fields.
 * Without either the call keeps its default (open confirmation or the Direct code's rights).
 */
data class RightsSelection(val preset: String? = null, val rights: JsonObject? = null) {
    val isEmpty: Boolean
        get() = preset == null && (rights == null || rights.isEmpty())

    /** Adds `preset` and `rights` to the arguments of a call. */
    fun addTo(builder: JsonObjectBuilder) {
        preset?.let { builder.put("preset", it) }
        rights?.takeIf { it.isNotEmpty() }?.let { builder.put("rights", it) }
    }

    companion object {
        fun preset(key: String) = RightsSelection(preset = key)
    }
}

/**
 * The JSON of [fields] of [rights] (`null`: every field). [RIGHTS_FIELD_VALID_UNTIL] without a value
 * is sent as `null`, which removes the end.
 */
fun rightsFieldsJson(rights: Rights, fields: Collection<String>? = null): JsonObject = buildJsonObject {
    fun wanted(field: String) = fields == null || field in fields
    if (wanted(RIGHTS_FIELD_FOLDERS)) {
        put(
            RIGHTS_FIELD_FOLDERS,
            buildJsonObject {
                put("kind", rights.folders.kind)
                if (rights.folders.kind == FOLDERS_SELECTED) {
                    put("paths", JsonArray(rights.folders.paths.map { JsonPrimitive(it) }))
                    put("connections", rights.folders.connections ?: false)
                }
            },
        )
    }
    if (wanted(RIGHTS_FIELD_WRITE)) put(RIGHTS_FIELD_WRITE, rights.write)
    listOf(RIGHTS_FIELD_COMMANDS, RIGHTS_FIELD_TERMINAL, RIGHTS_FIELD_SCREEN_VIEW, RIGHTS_FIELD_SCREEN_CONTROL).forEach { field ->
        if (wanted(field)) put(field, rights.decision(field))
    }
    if (wanted(RIGHTS_FIELD_VALID_UNTIL)) {
        val until = rights.validUntil
        if (until == null) put(RIGHTS_FIELD_VALID_UNTIL, JsonNull) else put(RIGHTS_FIELD_VALID_UNTIL, until)
    }
    if (wanted(RIGHTS_FIELD_ACCOUNT)) put(RIGHTS_FIELD_ACCOUNT, rights.account)
}

/** Arguments of `share.confirmRights`. */
internal fun confirmRightsArgs(target: RightsTarget, selection: RightsSelection): JsonObject {
    if (selection.isEmpty) throw CoreException("invalid", "Voreinstellung oder Rechte fehlen.")
    return buildJsonObject {
        put("target", target.toJson())
        selection.addTo(this)
    }
}

/** Arguments of `share.cancelRights`. */
internal fun cancelRightsArgs(target: RightsTarget): JsonObject = buildJsonObject { put("target", target.toJson()) }

/** Arguments of `share.setCodeRights`; [clear] removes the rights (devices then wait without rights). */
internal fun codeRightsArgs(selection: RightsSelection?, clear: Boolean): JsonObject {
    if (!clear && (selection == null || selection.isEmpty)) throw CoreException("invalid", "Voreinstellung, Rechte oder Entfernen fehlen.")
    return buildJsonObject {
        if (clear) {
            put("clear", true)
        } else {
            selection?.addTo(this)
        }
    }
}

/** Suspending wrappers of api.md §5.1; every call throws [CoreException] on errors. */
object ShareRightsApi {
    /** The presets, in the order the bridge names them. */
    suspend fun presets(): List<RightsPresetInfo> = Core.request<PresetsAnswer>("share.rightsPresets").presets

    /** The open confirmations and the rights of the Direct code. */
    suspend fun pending(): PendingRightsAnswer = Core.request<PendingRightsAnswer>("share.pendingRights")

    /**
     * Gives [target] exactly the rights of [selection] (the current ones where it names nothing);
     * the open confirmation ends. `conflict` for another identity, a deactivated grant or another
     * account than the app's.
     */
    suspend fun confirm(target: RightsTarget, selection: RightsSelection): RightsChange =
        saved(Core.request<RightsChange>("share.confirmRights", confirmRightsArgs(target, selection)))

    /** „Abbrechen“: the connection stays, without rights. */
    suspend fun cancel(target: RightsTarget): RightsChange =
        saved(Core.request<RightsChange>("share.cancelRights", cancelRightsArgs(target)))

    /** Sets the rights of the Direct code (they also apply to automatic acceptance). */
    suspend fun setCodeRights(selection: RightsSelection): RightsChange =
        saved(Core.request<RightsChange>("share.setCodeRights", codeRightsArgs(selection, clear = false)))

    /** Removes the Direct code's rights: a device that connects waits for a confirmation. */
    suspend fun clearCodeRights(): RightsChange =
        saved(Core.request<RightsChange>("share.setCodeRights", codeRightsArgs(null, clear = true)))

    private fun saved(change: RightsChange): RightsChange {
        if (!change.persisted) throw CoreException("internal", "Die Rechte wurden nicht als gespeichert bestätigt. Bitte erneut laden.")
        return change
    }
}
