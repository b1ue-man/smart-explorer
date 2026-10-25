package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** The same bounded worker route facts as Desktop, CLI and MCP. No network probe. */
@Serializable
data class RouteStatus(
    val available: Boolean,
    val peers: List<DeviceRoute> = emptyList(),
    val reason: String? = null,
    val error: String? = null,
    val truncated: Boolean = false,
    val measurementNote: String? = null,
)

@Serializable
data class DeviceRoute(
    val deviceId: String,
    val publicKey: String,
    val contactId: String? = null,
    val connections: List<ConnectionRoute> = emptyList(),
    val measurements: List<RouteMeasurement> = emptyList(),
)

@Serializable
data class ConnectionRoute(
    val connectionId: String,
    val protocol: String,
    val generation: Long,
    val revision: Long,
    val pauseEpoch: Long,
    val state: String,
    val label: String,
    val pathClass: String? = null,
    val applicationAllowed: Boolean,
    val reason: String,
    val remoteGuarantee: String,
    val remoteLimit: String? = null,
    val budget: RouteBudget? = null,
)

@Serializable
data class RouteBudget(
    val initialPtoMs: Long,
    val bestPtoMs: Long,
    val collectionMs: Long,
    val retryIntervalsMs: List<Long>,
    val maxProbeAttempts: Int? = null,
    val responseMs: Long,
    val validationMs: Long,
    /** Offset from this generation's start, not a wall-clock timestamp. */
    val directDeadlineMs: Long? = null,
    val frozenCandidates: Int? = null,
)

@Serializable
data class RouteMeasurement(
    val observedAtUnixMs: Long? = null,
    val generation: Long,
    val pathClass: String,
    val rttUs: Long? = null,
    val serializedTxFramePayloadBytes: Long,
    val authenticatedRxFramePayloadBytes: Long,
    val elapsedMs: Long,
)

object RouteApi {
    /** Exact selector: a saved contact, or device id together with its pinned key. */
    suspend fun status(contactId: String? = null, deviceId: String? = null, publicKey: String? = null): RouteStatus =
        Core.request("share.routeStatus", buildJsonObject {
            if (contactId != null) put("contactId", contactId) else {
                deviceId?.let { put("deviceId", it) }
                publicKey?.let { put("publicKey", it) }
            }
        })
}
