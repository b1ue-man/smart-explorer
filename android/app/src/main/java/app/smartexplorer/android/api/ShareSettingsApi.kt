package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * Share settings the desktop page „Geräte & Freigaben“ and `se share …` change, with the same core
 * transaction (api.md §5): per device connect/open automatically and trust reset, room name and
 * rejoining, a member's cached presence, the order of exports, the reachability of the Direct
 * code, a new Direct code, and finding paired devices in the local network. Every call throws
 * `CoreException` on errors.
 */
object ShareSettingsApi {
    /** `null` keeps a value; at least one must be set. */
    suspend fun setContactOptions(contactId: String, autoConnect: Boolean?, autoOpen: Boolean?) {
        Core.call(
            "share.setContactOptions",
            buildJsonObject {
                put("contactId", contactId)
                autoConnect?.let { put("autoConnect", it) }
                autoOpen?.let { put("autoOpen", it) }
            },
        )
    }

    /** Forgets the pinned identity and withdraws its access (commands off); the device stays. */
    suspend fun resetContactTrust(contactId: String) {
        Core.call("share.resetContactTrust", buildJsonObject { put("contactId", contactId) })
    }

    /** Empty [name] is „Raum“. */
    suspend fun renameRoom(profileId: String, name: String) {
        Core.call(
            "share.renameRoom",
            buildJsonObject {
                put("profileId", profileId)
                put("name", name)
            },
        )
    }

    /** A left room is joined automatically again. */
    suspend fun rejoinRoom(profileId: String) {
        Core.call("share.rejoinRoom", buildJsonObject { put("profileId", profileId) })
    }

    suspend fun resetMemberTrust(profileId: String, deviceId: String) {
        Core.call(
            "share.resetMemberTrust",
            buildJsonObject {
                put("profileId", profileId)
                put("deviceId", deviceId)
            },
        )
    }

    /** [offset] -1 = one place up, 1 = one place down; [scope] `direct` or a room profile id. */
    suspend fun moveExport(scope: String, path: String, offset: Int) {
        Core.call(
            "share.moveExport",
            buildJsonObject {
                put("scope", scope)
                put("path", path)
                put("offset", offset)
            },
        )
    }

    /** Direct code reachable for new contacts until the worker restarts. */
    suspend fun setDirectOnline(online: Boolean) {
        Core.call("share.setDirectOnline", buildJsonObject { put("online", online) })
    }

    suspend fun setLanPresence(enabled: Boolean) {
        Core.call("share.setLanPresence", buildJsonObject { put("enabled", enabled) })
    }

    /** A new Direct code; returns it. Direct authorizations pause until confirmed again. */
    suspend fun renewCode(): String = Core.request<RenewedCode>("share.renewCode").directCode

    @Serializable
    private data class RenewedCode(val directCode: String = "", val warning: String? = null)
}
