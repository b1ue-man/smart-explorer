package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

@Serializable
data class ShareServerInfo(
    val server: String,
    val security: String,
    val summary: String = "",
    val plaintext: Boolean = false,
    val ignoredPlaintext: Int = 0,
    val migrated: Boolean = false,
)

@Serializable
data class UnconfirmedPairing(
    val exchangeId: String,
    val kind: String,
    val contactId: String? = null,
    val roomProfileId: String? = null,
    val label: String = "",
    val revocable: Boolean = false,
)

object ShareSecurityApi {
    suspend fun serverInfo(): ShareServerInfo = Core.request("share.serverInfo")

    suspend fun setServer(server: String, allowPlaintext: Boolean): ShareServerInfo = Core.request(
        "share.setServer", buildJsonObject {
            put("server", server)
            put("allowPlaintext", allowPlaintext)
        },
    )

    suspend fun suggestPin(): String = Core.request<PinAnswer>("share.suggestPin").pin

    suspend fun unconfirmedPairings(): List<UnconfirmedPairing> =
        Core.request<PairingAnswer>("share.unconfirmedPairings").pairings

    suspend fun resolvePairing(exchangeId: String, revoke: Boolean) {
        Core.call("share.resolvePairing", buildJsonObject {
            put("exchangeId", exchangeId)
            put("revoke", revoke)
        })
    }

    @Serializable
    private data class PinAnswer(val pin: String)

    @Serializable
    private data class PairingAnswer(val pairings: List<UnconfirmedPairing>)
}
