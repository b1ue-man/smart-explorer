package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

const val SHARE_READ_ONLY = "read_only"
const val SHARE_READ_WRITE = "read_write"

@Serializable
data class ShareRoomPolicy(val membersMayWrite: Boolean, val confirmNewMembers: Boolean)

@Serializable
data class ShareConnectionExport(val account: String, val access: String)

@Serializable
data class ShareConnectionExports(
    val direct: List<ShareConnectionExport> = emptyList(),
    val rooms: Map<String, List<ShareConnectionExport>> = emptyMap(),
)

/** Pins are the exact identity from the native snapshot, including an empty legacy nodeId. */
@Serializable
data class ShareWriteGrant(
    val deviceId: String,
    val name: String = "",
    val publicKey: String,
    val nodeId: String,
    val fingerprint: String,
    val state: String,
    val write: Boolean,
    val active: Boolean,
    val canSetWrite: Boolean,
)

@Serializable
data class ShareHomeMigration(val scope: String, val path: String)

@Serializable
data class ShareSavedConnection(
    val account: String,
    val label: String = "",
    val shared: Boolean = false,
    val access: String? = null,
)

@Serializable
data class ShareConnections(
    val connections: List<ShareSavedConnection> = emptyList(),
    val sharedConnections: List<ShareConnectionExport> = emptyList(),
    val warning: String = "",
)

@Serializable
data class ShareRequestPolicy(val requests: String = "Ask", val warning: String? = null)

/** Persisted is the profile commit, not a synchronous transport/session barrier. */
@Serializable
private data class PolicySaved(val persisted: Boolean, val changed: Boolean = false)

/** FC1 mutations preserve stored path/account bytes and every unrelated permission. */
object SharePolicyApi {
    suspend fun connections(scope: String): ShareConnections = Core.request(
        "share.connections", buildJsonObject { put("scope", scope) },
    )

    suspend fun policy(): ShareRequestPolicy = Core.request("share.policy")

    suspend fun setPolicy(requests: String) {
        save("share.setPolicy", buildJsonObject { put("requests", requests) })
    }

    suspend fun allowGrantAgain(grant: ShareWriteGrant) {
        save("share.allowGrantAgain", buildJsonObject {
            put("deviceId", grant.deviceId)
            put("name", grant.name)
            put("publicKey", grant.publicKey)
            put("nodeId", grant.nodeId)
            put("fingerprint", grant.fingerprint)
        })
    }

    suspend fun withdrawGrant(grant: ShareWriteGrant) {
        save("share.withdrawGrant", buildJsonObject {
            put("deviceId", grant.deviceId)
            put("name", grant.name)
            put("publicKey", grant.publicKey)
            put("nodeId", grant.nodeId)
            put("fingerprint", grant.fingerprint)
        })
    }

    suspend fun setRoomMember(room: ShareRoom, member: ShareMember, action: String) {
        val key = member.publicKey ?: throw CoreException("invalid", "Mitgliedsschlüssel fehlt; bitte Status neu laden.")
        val node = member.nodeId ?: throw CoreException("invalid", "Mitgliedsknoten fehlt; bitte Status neu laden.")
        val fingerprint = member.fingerprint ?: throw CoreException("invalid", "Mitgliedsfingerabdruck fehlt; bitte Status neu laden.")
        save("share.setRoomMember", buildJsonObject {
            put("profileId", room.profileId)
            put("roomId", room.roomId)
            put("deviceId", member.deviceId)
            put("name", member.name)
            put("publicKey", key)
            put("nodeId", node)
            put("fingerprint", fingerprint)
            put("action", action)
        })
    }

    suspend fun setExportAccess(scope: String, path: String, access: String, allowSystemWrites: Boolean? = null, expectedAccess: String? = null) {
        save("share.setExportAccess", buildJsonObject {
            put("scope", scope)
            put("path", path)
            put("access", access)
            allowSystemWrites?.let { put("allowSystemWrites", it) }
            expectedAccess?.let { put("expectedAccess", it) }
        })
    }

    suspend fun setConnectionExport(scope: String, account: String, shared: Boolean, access: String? = null, expectedShared: Boolean? = null) {
        save("share.setConnectionExport", buildJsonObject {
            put("scope", scope)
            put("account", account)
            put("shared", shared)
            if (shared) access?.let { put("access", it) }
            expectedShared?.let { put("expectedShared", it) }
        })
    }

    suspend fun setContactWrite(grant: ShareWriteGrant, write: Boolean) {
        save("share.setContactWrite", buildJsonObject {
            put("deviceId", grant.deviceId)
            put("publicKey", grant.publicKey)
            put("nodeId", grant.nodeId)
            put("fingerprint", grant.fingerprint)
            put("name", grant.name)
            put("write", write)
        })
    }

    suspend fun setShareBack(contactId: String, shareBack: Boolean) {
        save("share.setShareBack", buildJsonObject {
            put("contactId", contactId)
            put("shareBack", shareBack)
        })
    }

    suspend fun setRoomPolicy(room: ShareRoom, membersMayWrite: Boolean? = null, confirmNewMembers: Boolean? = null) {
        save("share.setRoomPolicy", buildJsonObject {
            put("profileId", room.profileId)
            put("roomId", room.roomId)
            membersMayWrite?.let { put("membersMayWrite", it) }
            confirmNewMembers?.let { put("confirmNewMembers", it) }
        })
    }

    private suspend fun save(method: String, args: JsonObject) {
        val result = Core.request<PolicySaved>(method, args)
        if (!result.persisted) throw CoreException("internal", "Die Rechte wurden nicht als gespeichert bestätigt. Bitte erneut laden.")
    }
}
