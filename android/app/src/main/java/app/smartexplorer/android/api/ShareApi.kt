package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// Answers of api.md §5 (Share); field names exactly as the contract.

/** `ShareStatus.identity` */
@Serializable
data class ShareIdentityInfo(
    val deviceId: String = "",
    val deviceName: String = "",
    val fingerprint: String = "",
    val directCode: String = "",
)

/** A paired Direct device; [location] is its `share://` place. */
@Serializable
data class ShareDevice(
    val contactId: String,
    val name: String = "",
    val status: String = "",
    val statusText: String = "",
    val online: Boolean = false,
    val location: String = "",
    val lan: Boolean = false,
)

@Serializable
data class ShareMember(
    val deviceId: String,
    val name: String = "",
    val status: String = "",
    val location: String = "",
    val blocked: Boolean = false,
)

@Serializable
data class ShareRoom(
    val profileId: String,
    val roomId: String = "",
    val name: String = "",
    val status: String = "",
    val autoJoin: Boolean = false,
    val location: String? = null,
    val members: List<ShareMember> = emptyList(),
)

/** api.md `Request` (named apart from `CoreEvent.ShareRequest`). */
@Serializable
data class ShareRequestInfo(
    val requestId: String,
    val contactId: String? = null,
    val name: String = "",
    val stateText: String = "",
    val canAccept: Boolean = false,
    val canReject: Boolean = false,
    val canRetry: Boolean = false,
    val canDelete: Boolean = false,
    val message: String? = null,
    val timeMs: Long = 0,
)

/** A local folder this device offers. */
@Serializable
data class ShareExport(val label: String = "", val path: String)

/** Exports for Direct devices and per room (`rooms` keyed by room profile id). */
@Serializable
data class ShareExports(
    val direct: List<ShareExport> = emptyList(),
    val rooms: Map<String, List<ShareExport>> = emptyMap(),
)

/** Own running offer; [target] is `direct` or a room profile id. */
@Serializable
data class DiscoveryOffer(
    val offerId: String,
    val target: String = SHARE_DIRECT,
    val alias: String = "",
    val untilMs: Long = 0,
)

/** A device or room that is discoverable right now; [kind] `direct|room`. */
@Serializable
data class DiscoveryAdvert(
    val discoveryId: String,
    val kind: String = SHARE_DIRECT,
    val alias: String = "",
    val expiresMs: Long = 0,
    val compatible: Boolean = true,
)

/** Latest PIN pairing; [state] `running|done|failed|canceled`. */
@Serializable
data class DiscoveryExchange(
    val exchangeId: String,
    val state: String = "running",
    val message: String? = null,
)

@Serializable
data class ShareDiscovery(
    val offer: DiscoveryOffer? = null,
    val advertisements: List<DiscoveryAdvert> = emptyList(),
    val exchange: DiscoveryExchange? = null,
)

/** A removed Direct device that may be readmitted. */
@Serializable
data class RemovedDevice(val deviceId: String, val name: String = "")

/** `share.status`: the poller's latest snapshot. */
@Serializable
data class ShareStatus(
    val running: Boolean = false,
    val connected: Boolean = false,
    val relayUrl: String? = null,
    val lastError: String? = null,
    /** `null` = no Share server configured (LAN only). */
    val server: String? = null,
    val lanPresence: String = "",
    val identity: ShareIdentityInfo = ShareIdentityInfo(),
    val devices: List<ShareDevice> = emptyList(),
    /** This phone as exec host (ShareExecApi.kt). */
    val execProvider: ExecProviderInfo = ExecProviderInfo(),
    val execTargets: List<ExecTarget> = emptyList(),
    val rooms: List<ShareRoom> = emptyList(),
    val incoming: List<ShareRequestInfo> = emptyList(),
    val outgoing: List<ShareRequestInfo> = emptyList(),
    val exports: ShareExports = ShareExports(),
    val discovery: ShareDiscovery = ShareDiscovery(),
    val removedDevices: List<RemovedDevice> = emptyList(),
    val notices: List<String> = emptyList(),
)

/** `share.createRoom` */
@Serializable
data class RoomCreated(val profileId: String, val code: String)

/** Result of the `share.exec` task (output at most 1 MiB). */
@Serializable
data class ExecResult(
    val stdout: String = "",
    val stderr: String = "",
    val exitCode: Int? = null,
    val timedOut: Boolean = false,
    val truncated: Boolean = false,
)

/** Discovery target and export scope for "all Direct devices" (otherwise a room profile id). */
const val SHARE_DIRECT = "direct"

/** Suspending wrappers for api.md §5; every call throws [CoreException] on errors. */
object ShareApi {
    suspend fun status(): ShareStatus = Core.request<ShareStatus>("share.status")

    /** `true` while the Share page is visible: the core polls the worker faster. */
    suspend fun watch(active: Boolean) {
        Core.call("share.watch", buildJsonObject { put("active", active) })
    }

    /** Empty [server] removes the Share server (LAN only). */
    suspend fun setServer(server: String) {
        Core.call("share.setServer", buildJsonObject { put("server", server) })
    }

    suspend fun setOnline(online: Boolean) {
        Core.call("share.setOnline", buildJsonObject { put("online", online) })
    }

    suspend fun setName(name: String) {
        Core.call("share.setName", buildJsonObject { put("name", name) })
    }

    /** [target] `direct` or a room profile id. */
    suspend fun discoverable(target: String, alias: String, pin: String, minutes: Int) {
        Core.call(
            "share.discoverable",
            buildJsonObject {
                put("target", target)
                put("alias", alias)
                put("pin", pin)
                put("minutes", minutes)
            },
        )
    }

    suspend fun stopDiscoverable(offerId: String) {
        Core.call("share.stopDiscoverable", buildJsonObject { put("offerId", offerId) })
    }

    /** Results appear in `ShareStatus.discovery.advertisements`. */
    suspend fun discover() {
        Core.call("share.discover")
    }

    suspend fun connect(discoveryId: String, pin: String) {
        Core.call(
            "share.connect",
            buildJsonObject {
                put("discoveryId", discoveryId)
                put("pin", pin)
            },
        )
    }

    suspend fun cancelConnect(exchangeId: String) {
        Core.call("share.cancelConnect", buildJsonObject { put("exchangeId", exchangeId) })
    }

    /** Adds a Direct device by its code; returns the contact id. */
    suspend fun addDirect(code: String, name: String): String = Core.request<ContactRef>(
        "share.addDirect",
        buildJsonObject {
            put("code", code)
            put("name", name)
        },
    ).contactId

    suspend fun removeDevice(contactId: String): EndpointRemoval =
        Core.request<EndpointRemoval>("share.removeDevice", buildJsonObject { put("contactId", contactId) })

    suspend fun readmit(deviceId: String) {
        Core.call("share.readmit", buildJsonObject { put("deviceId", deviceId) })
    }

    suspend fun createRoom(name: String): RoomCreated =
        Core.request<RoomCreated>("share.createRoom", buildJsonObject { put("name", name) })

    /** Returns the new room's profile id. */
    suspend fun joinRoom(code: String, name: String): String = Core.request<ProfileRef>(
        "share.joinRoom",
        buildJsonObject {
            put("code", code)
            put("name", name)
        },
    ).profileId

    /** Invite code of an existing room, `null` when none is available. */
    suspend fun roomCode(profileId: String): String? =
        Core.request<RoomCodeAnswer>("share.roomCode", profileArgs(profileId)).code

    suspend fun leaveRoom(profileId: String) {
        Core.call("share.leaveRoom", profileArgs(profileId))
    }

    suspend fun removeRoom(profileId: String): EndpointRemoval =
        Core.request<EndpointRemoval>("share.removeRoom", profileArgs(profileId))

    suspend fun requestAccess(contactId: String, message: String?) {
        Core.call(
            "share.requestAccess",
            buildJsonObject {
                put("contactId", contactId)
                put("message", message)
            },
        )
    }

    suspend fun decide(requestId: String, accept: Boolean) {
        Core.call(
            "share.decide",
            buildJsonObject {
                put("requestId", requestId)
                put("accept", accept)
            },
        )
    }

    suspend fun retry(requestId: String) {
        Core.call("share.retry", buildJsonObject { put("requestId", requestId) })
    }

    suspend fun deleteRequest(requestId: String) {
        Core.call("share.deleteRequest", buildJsonObject { put("requestId", requestId) })
    }

    /** [scope] `direct` or a room profile id; [path] must be an existing local folder. */
    suspend fun addExport(scope: String, path: String, label: String?) {
        Core.call(
            "share.addExport",
            buildJsonObject {
                put("scope", scope)
                put("path", path)
                put("label", label)
            },
        )
    }

    suspend fun removeExport(scope: String, path: String) {
        Core.call(
            "share.removeExport",
            buildJsonObject {
                put("scope", scope)
                put("path", path)
            },
        )
    }

    /** Runs [command] on the device behind [location]; the task result is an [ExecResult]. */
    suspend fun exec(location: String, command: String, shell: Boolean, timeoutSecs: Int): String = Core.request<TaskId>(
        "share.exec",
        buildJsonObject {
            put("location", location)
            put("command", command)
            put("shell", shell)
            put("timeoutSecs", timeoutSecs)
        },
    ).taskId

    private fun profileArgs(profileId: String): JsonObject = buildJsonObject { put("profileId", profileId) }

    // Response shapes used only here (nested, so equally named helpers of other api files never clash).

    @Serializable
    private data class ContactRef(val contactId: String)

    @Serializable
    private data class ProfileRef(val profileId: String)

    @Serializable
    private data class RoomCodeAnswer(val code: String? = null)

    @Serializable
    private data class TaskId(val taskId: String)
}
