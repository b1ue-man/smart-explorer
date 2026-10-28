package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.screen.ScreenIconRef
import app.smartexplorer.android.ui.screen.ScreenTarget
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.util.UUID

// These DTOs preserve the versioned native family spelling, including icon references.
@Serializable data class FamilyIdentity(val session: String, val generation: Long)
@Serializable data class FamilyGeometry(val x: Int, val y: Int, val width: Int, val height: Int)
@Serializable data class FamilyOperations(val resize: Boolean, val focus: Boolean, val close: Boolean, val input: Boolean)
@Serializable data class FamilyWindow(val id: String, val title: String, val app: String, val app_id: String? = null, val icon: ScreenIconRef? = null)
@Serializable data class FamilyListEntry(val source: String, val identity: FamilyIdentity, val window: FamilyWindow, val launch_request: String? = null)
@Serializable data class FamilyListing(val version: Int, val entries: List<FamilyListEntry>, val limitations: List<String>)
@Serializable data class FamilyMember(
    val identity: FamilyIdentity, val owner: FamilyIdentity? = null, val app_id: String? = null,
    val app: String, val title: String, val icon: ScreenIconRef? = null, val kind: String,
    val geometry: FamilyGeometry, val z_order: Int, val focused: Boolean, val visible: Boolean,
    val minimized: Boolean, val operations: FamilyOperations,
)
@Serializable data class FamilySnapshot(
    val version: Int, val session: String, val revision: Long, val layout_revision: Long,
    val root: FamilyIdentity, val members: List<FamilyMember>, val limitations: List<String>,
)
@Serializable data class FamilyOperationResult(val request_id: String, val status: String? = null, val reason: String? = null, val snapshot: FamilySnapshot? = null)
@Serializable data class FamilyState(val snapshot: FamilySnapshot? = null, val notice: String? = null, val last_operation: FamilyOperationResult? = null)
@Serializable data class FamilyRequestReply(val request_id: String)
/** The background service's feed of one device (design 5.2); `control`: the device lets this one steer. */
@Serializable data class DeviceFeed(
    val known: Boolean = false,
    val present: Boolean = false,
    val families: FamilyListing? = null,
    val familiesUnavailable: String? = null,
    val servicesUnavailable: String? = null,
    val rdp: RdpFeedState? = null,
    val unavailable: String? = null,
    val control: Boolean = false,
)

object FamilyApi {
    /** Windows, services and Remote Desktop state of a device without any request to it. */
    suspend fun feed(deviceId: String, publicKey: String): DeviceFeed = Core.request("screen.feed", buildJsonObject {
        put("deviceId", deviceId); put("publicKey", publicKey)
    })
    suspend fun list(target: ScreenTarget): FamilyListing = Core.request("screen.families", buildJsonObject {
        put("relation", target.relation); put("relationId", target.relationId); target.deviceId?.let { put("deviceId", it) }
        target.expectedNodeId?.let { put("expectedNodeId", it) }
    })
    suspend fun state(session: Long): FamilyState = Core.request("screen.viewFamilyState", sessionArgs(session))
    suspend fun refresh(session: Long) { Core.request<JsonObject>("screen.viewFamilySnapshot", sessionArgs(session)) }
    suspend fun operate(session: Long, identity: FamilyIdentity, operation: String, width: Int? = null, height: Int? = null): FamilyRequestReply =
        Core.request("screen.viewFamilyOperation", buildJsonObject {
            put("session", session)
            put("request", buildJsonObject {
                put("request_id", UUID.randomUUID().toString())
                put("identity", Core.json.encodeToJsonElement(FamilyIdentity.serializer(), identity))
                put("operation", buildJsonObject {
                    put("operation", operation)
                    width?.let { put("width", it) }; height?.let { put("height", it) }
                })
            })
        })
    private fun sessionArgs(session: Long) = buildJsonObject { put("session", session) }
    fun validate(entry: FamilyListEntry) {
        require(entry.identity.generation == 1L && entry.identity.session.matches(Regex("[0-9a-f]{32}")) &&
            entry.source == "window:@family.${entry.identity.session}") { "Ungültige bestätigte Fensterauswahl." }
    }
}
