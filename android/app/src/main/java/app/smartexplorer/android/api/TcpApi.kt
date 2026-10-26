package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

const val TCP_LOCAL_ACCESS_NOTICE = "Jeder gleichzeitig lokal laufende Prozess kann diesen Loopbackport erreichen. Der weitergeleitete Dienst muss seine Benutzer selbst authentifizieren; ohne eigene Authentifizierung erhält dieser Prozess Zugriff auf den Dienst."

@Serializable
data class TcpTarget(
    val relation: String, val relationId: String, val deviceId: String,
    val publicKey: String, val nodeId: String, val name: String, val relationLabel: String,
    val canStart: Boolean = false,
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
    @SerialName("expires_at") val expiresAt: Long,
    @SerialName("active_connections") val activeConnections: Int,
    @SerialName("socket_read_bytes") val socketReadBytes: Long,
    @SerialName("socket_written_bytes") val socketWrittenBytes: Long,
    val closing: Boolean = false,
    @SerialName("last_error") val lastError: String? = null,
)
@Serializable private data class TcpTargets(val devices: List<TcpTarget> = emptyList())
@Serializable private data class TcpTunnels(val tunnels: List<TcpTunnel> = emptyList())
@Serializable private data class TcpOpened(val tunnel: TcpTunnel)

object TcpApi {
    suspend fun devices(): List<TcpTarget> = Core.request<TcpTargets>("tcp.devices").devices
    suspend fun list(): List<TcpTunnel> = Core.request<TcpTunnels>("tcp.list").tunnels
    suspend fun status(id: String): TcpTunnel = Core.request<TcpOpened>("tcp.status", buildJsonObject { put("id", id) }).tunnel
    suspend fun close(id: String) { Core.call("tcp.close", buildJsonObject { put("id", id) }) }
    suspend fun start(target: TcpTarget, host: String, port: Int, localPort: Int, lifetimeSecs: Int, acknowledgeLocalAccess: Boolean): TcpTunnel =
        Core.request<TcpOpened>("tcp.start", buildJsonObject {
            put("relation", target.relation); put("relationId", target.relationId)
            put("deviceId", target.deviceId); put("publicKey", target.publicKey); put("nodeId", target.nodeId)
            put("targetHost", host); put("targetPort", port); put("localPort", localPort)
            put("lifetimeSecs", lifetimeSecs); put("acknowledgeLocalAccess", acknowledgeLocalAccess)
        }).tunnel
}
