package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.NativeBridge
import java.nio.ByteBuffer
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

/** An explicit administrative machine target, never a normal ScreenDevice. */
data class LoginTarget(val body: JsonObject) {
    val machineId: String get() = body["machine_id"]?.jsonPrimitive?.content.orEmpty()
    val publicKey: String get() = body["public_key"]?.jsonPrimitive?.content.orEmpty()
}
@Serializable data class LoginToken(val frame_id: Long, val width: Int, val height: Int, val digest: String)
@Serializable data class LoginRights(val screen_view: Boolean = false, val screen_control: Boolean = false,
    val login: Boolean = false, val before_login: Boolean = false, val locked: Boolean = false, val uac: Boolean = false, val sas: Boolean = false)
@Serializable data class LoginAccess(val rights: LoginRights = LoginRights(), val policy_revision: Long = 0,
    val target_user: String = "", val expires_at: Long = 0)
@Serializable data class LoginBinding(val session_id: String = "", val user_id: String? = null, val session_generation: String = "",
    val desktop_generation: String = "", val desktop: String = "unavailable")
@Serializable data class LoginChallenge(val boot_generation: String = "", val binding: LoginBinding = LoginBinding(), val policy_revision: Long = 0)
@Serializable data class LoginView(val session: String = "", val target_user: String = "", val challenge: LoginChallenge? = null,
    val access: LoginAccess? = null, val state: String = "connecting")
@Serializable data class LoginState(val state: String = "connecting", val reason: String? = null, val view: LoginView? = null,
    val frame: LoginToken? = null, val input_ready: Boolean = false, val control: Boolean = false, val focused: Boolean = false, val presented_frame: LoginToken? = null, val presentation_epoch: Long = 0, val notice: String? = null)
@Serializable private data class LoginOpened(val session: Long)
object LoginApi {
    suspend fun importTarget(text: String): LoginTarget {
        require(text.toByteArray(Charsets.UTF_8).size <= 32768) { "Maschinenziel überschreitet 32 KiB." }
        val supplied = Core.json.parseToJsonElement(text) as? JsonObject ?: error("Maschinenziel muss ein JSON-Objekt sein.")
        val result = Core.call("login.target", buildJsonObject { put("target", supplied) }) as JsonObject
        return LoginTarget(result["target"] as JsonObject)
    }
    suspend fun open(target: LoginTarget, user: String, acknowledged: Boolean): Long = Core.request<LoginOpened>("login.viewOpen", buildJsonObject {
        put("target", target.body); put("target_user", user); put("acknowledge_machine_pin", acknowledged)
    }).session
    suspend fun status(id: Long): LoginState = Core.request("login.viewStatus", args(id))
    suspend fun localStatus(): JsonObject = Core.call("login.status") as JsonObject
    suspend fun visible(id: Long) { Core.call("login.viewVisible", args(id) { put("visible", true) }) }
    suspend fun focus(id: Long, focused: Boolean, control: Boolean): LoginState = Core.request("login.viewFocus", args(id) { put("focused", focused); put("control", control) })
    suspend fun presented(id: Long, token: LoginToken, epoch: Long) { Core.call("login.viewPresented", args(id) { put("token", Core.json.encodeToJsonElement(LoginToken.serializer(), token)); put("presentation_epoch", epoch) }) }
    suspend fun input(id: Long, token: LoginToken, epoch: Long, events: List<JsonObject>) { Core.call("login.viewInput", args(id) {
        put("token", Core.json.encodeToJsonElement(LoginToken.serializer(), token)); put("presentation_epoch", epoch); put("events", JsonArray(events))
    }) }
    suspend fun sas(id: Long, token: LoginToken, epoch: Long) { Core.call("login.viewSas", args(id) { put("token", Core.json.encodeToJsonElement(LoginToken.serializer(), token)); put("presentation_epoch", epoch) }) }
    fun pull(id: Long, dst: ByteBuffer, wait: Int): Int = NativeBridge.loginPull(id, dst, wait)
    fun releaseFast(id: Long) { if (id > 0) NativeBridge.loginRelease(id) }
    fun closeFast(id: Long) { if (id > 0) NativeBridge.loginClose(id) }
    private fun args(id: Long, extra: kotlinx.serialization.json.JsonObjectBuilder.() -> Unit = {}) = buildJsonObject { put("session", id); extra() }
}
