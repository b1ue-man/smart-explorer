package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

@Serializable
data class RdpHost(
    val version: Int,
    val platform: String,
    val services: List<RdpService> = emptyList(),
    val unavailableReason: String? = null,
    val diagnostics: List<String> = emptyList(),
)
@Serializable
data class RdpService(
    val id: String, val kind: String, val version: String? = null,
    val sessionMode: String, val port: Int? = null, val running: Boolean,
    val tls: Boolean? = null, val nla: Boolean? = null,
    val certificateSha256: String? = null, val certificateSource: String? = null,
    val remoteApps: List<RdpRemoteApp> = emptyList(),
    val evidence: List<String> = emptyList(), val unavailableReason: String? = null,
)
@Serializable
data class RdpRemoteApp(val alias: String, val displayName: String, val collection: String)
@Serializable
data class RdpClient(
    val id: String, val kind: String, val version: String? = null,
    val certificatePolicy: String, val remoteApp: Boolean, val available: Boolean, val detail: String,
)
@Serializable
data class RdpClients(val clients: List<RdpClient> = emptyList(), val unavailableReason: String? = null)

/** Read-only on Android: no URI, client installation or TCP fallback. */
object RdpApi {
    suspend fun localHost(): RdpHost = Core.request("rdp.localProbe")
    suspend fun clients(): RdpClients = Core.request("rdp.clients")
    suspend fun probe(target: TcpTarget): RdpHost = Core.request("rdp.probe", buildJsonObject {
        put("relation", target.relation); put("relationId", target.relationId)
        put("deviceId", target.deviceId); put("publicKey", target.publicKey); put("nodeId", target.nodeId)
    })
}
