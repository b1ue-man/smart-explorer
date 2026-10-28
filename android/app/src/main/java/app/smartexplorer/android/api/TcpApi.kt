package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** Android cannot tell which app opens a local connection (design 4.1): a shared service's local port exists only after „Öffnen“. */
const val TCP_ANY_APP_NOTICE = "Solange der Zugang offen ist, erreicht jede App dieses Telefons den Dienst. Schließen beendet ihn."

@Serializable
data class TcpTarget(
    val relation: String, val relationId: String, val deviceId: String,
    val publicKey: String, val nodeId: String, val name: String, val relationLabel: String,
    val canStart: Boolean = false,
    /** The relation se uses for this device now (present, Direct before room). */
    val present: Boolean = false,
) {
    val key: String get() = "$relation/$relationId/$deviceId/$publicKey/$nodeId"
}
@Serializable
data class TcpTunnel(
    val id: String, val role: String,
    @SerialName("remote_node") val remoteNode: String,
    @SerialName("initiator_node") val initiatorNode: String,
    @SerialName("target_node") val targetNode: String,
    val direction: String,
    @SerialName("relation_kind") val relationKind: String,
    @SerialName("relation_id") val relationId: String,
    val destination: JsonObject,
    @SerialName("bound_target") val boundTarget: String? = null,
    val listener: String? = null,
    @SerialName("started_at") val startedAt: Long,
    /** `null`: no fixed lifetime; the tunnel lives as long as connection and grant. */
    @SerialName("expires_at") val expiresAt: Long? = null,
    /** Who may use a local listener: same_account, await_process, any_local; null for a target. */
    val admission: String? = null,
    @SerialName("active_connections") val activeConnections: Int,
    @SerialName("socket_read_bytes") val socketReadBytes: Long,
    @SerialName("socket_written_bytes") val socketWrittenBytes: Long,
    val closing: Boolean = false,
    @SerialName("last_error") val lastError: String? = null,
)
/** A locally listening service a device shares under „Bildschirm steuern“ (design 4.2/4.3). */
@Serializable
data class TcpService(
    val port: Int,
    val scope: String,
    val process: String? = null,
    val pid: Long? = null,
    /** The local access of this port while one exists. */
    val mapping: TcpMapping? = null,
)
/** How this phone reaches one shared service (design 4.3): the device's own loopback address. */
@Serializable
data class TcpMapping(
    @SerialName("host_port") val hostPort: Int,
    val process: String? = null,
    val address: String? = null,
    /** Why the local port differs from the device's („Port belegt“). */
    @SerialName("port_note") val portNote: String? = null,
    val url: String? = null,
    val tunnel: String? = null,
    @SerialName("last_error") val lastError: String? = null,
)
@Serializable private data class TcpMappingReply(val mapping: TcpMapping)
@Serializable
data class TcpServices(val services: List<TcpService> = emptyList(), val unavailable: String? = null, val control: Boolean = false)
@Serializable private data class TcpTargets(val devices: List<TcpTarget> = emptyList())
@Serializable private data class TcpTunnels(val tunnels: List<TcpTunnel> = emptyList())
@Serializable private data class TcpOpened(val tunnel: TcpTunnel)

object TcpApi {
    suspend fun devices(): List<TcpTarget> = Core.request<TcpTargets>("tcp.devices").devices
    suspend fun list(): List<TcpTunnel> = Core.request<TcpTunnels>("tcp.list").tunnels
    suspend fun status(id: String): TcpTunnel = Core.request<TcpOpened>("tcp.status", buildJsonObject { put("id", id) }).tunnel
    suspend fun close(id: String) { Core.call("tcp.close", buildJsonObject { put("id", id) }) }
    /** A further forwarding: no acknowledgement, no fixed lifetime (design 4.5). */
    suspend fun start(target: TcpTarget, host: String, port: Int, localPort: Int): TcpTunnel =
        Core.request<TcpOpened>("tcp.start", buildJsonObject {
            put("relation", target.relation); put("relationId", target.relationId)
            put("deviceId", target.deviceId); put("publicKey", target.publicKey); put("nodeId", target.nodeId)
            put("targetHost", host); put("targetPort", port); put("localPort", localPort)
        }).tunnel
    suspend fun services(deviceId: String, publicKey: String): TcpServices =
        Core.request("tcp.services", buildJsonObject { put("deviceId", deviceId); put("publicKey", publicKey) })
    /** Opens the local access to one shared service at the device's own loopback address. */
    suspend fun openService(deviceId: String, publicKey: String, port: Int): TcpMapping =
        Core.request<TcpMappingReply>("tcp.openService", buildJsonObject {
            put("deviceId", deviceId); put("publicKey", publicKey); put("port", port)
        }).mapping
    suspend fun closeService(deviceId: String, publicKey: String, port: Int) {
        Core.call("tcp.closeService", buildJsonObject { put("deviceId", deviceId); put("publicKey", publicKey); put("port", port) })
    }
}
