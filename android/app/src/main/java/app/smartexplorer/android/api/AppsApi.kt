package app.smartexplorer.android.api

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.AtomicFile
import android.util.Base64
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.screen.AppHostSession
import app.smartexplorer.android.screen.AppLaunchRequest
import app.smartexplorer.android.screen.HostCatalogApp
import app.smartexplorer.android.screen.ScreenIconRef
import app.smartexplorer.android.screen.ScreenIconReply
import java.io.File
import java.io.FileNotFoundException
import java.security.MessageDigest
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.encodeToJsonElement
import kotlinx.serialization.json.put

@Serializable
internal data class RemoteAppCatalog(
    val version: Int,
    @SerialName("device_id") val deviceId: String,
    val session: AppHostSession,
    val revision: String,
    @SerialName("authorization_scope") val authorizationScope: String,
    val apps: List<HostCatalogApp>,
    val limits: List<String> = emptyList(),
)

@Serializable
internal data class AppWindowBinding(
    @SerialName("app_id") val appId: String,
    @SerialName("user_id") val userId: String,
    @SerialName("session_id") val sessionId: String,
    @SerialName("window_id") val windowId: String,
    val generation: Long,
)

@Serializable
internal data class RemoteAppReceipt(
    val request: AppLaunchRequest,
    @SerialName("launch_id") val launchId: String,
    val state: String,
    val reason: String,
    val pid: Long? = null,
    val window: AppWindowBinding? = null,
)

@Serializable
private data class AppsResult(
    val kind: String,
    val catalog: RemoteAppCatalog? = null,
    val receipt: RemoteAppReceipt? = null,
    val reply: ScreenIconReply? = null,
)
@Serializable
private data class AppsReply(val nodeId: String, val deviceId: String, val result: AppsResult)

@Serializable
internal data class SavedRemoteApp(
    val endpoint: String,
    val publicKey: String,
    val name: String,
    val request: AppLaunchRequest,
)

/** Only a fresh authenticated device reply can construct this UI selection. */
internal class ConfirmedRemoteAppWindow private constructor(
    val target: TcpTarget, val nodeId: String, val request: AppLaunchRequest, val window: AppWindowBinding,
) {
    companion object {
        internal fun from(target: TcpTarget, nodeId: String, receipt: RemoteAppReceipt): ConfirmedRemoteAppWindow? {
            val window = receipt.window ?: return null
            val request = receipt.request
            if (receipt.state !in setOf("started", "started_window_pending", "activated") ||
                request.targetDeviceId != target.deviceId || window.appId != request.appId ||
                window.userId != request.userId || window.sessionId != request.sessionId ||
                window.windowId.isBlank() || window.generation <= 0 || nodeId.isBlank()) return null
            return ConfirmedRemoteAppWindow(target, nodeId, request, window)
        }
    }
}
internal data class AppReceiptResult(val receipt: RemoteAppReceipt?, val window: ConfirmedRemoteAppWindow?)

@Serializable
internal data class LocalAppGrantAccess(
    val capabilities: Map<String, String> = emptyMap(),
    @SerialName("valid_until") val validUntil: Long? = null,
    val expired: Boolean = false,
    val constraints: JsonObject,
)
@Serializable
internal data class LocalAppRelation(
    val target: JsonObject,
    @SerialName("relation_kind") val relationKind: String,
    @SerialName("relation_id") val relationId: String,
    @SerialName("relation_name") val relationName: String,
    val active: Boolean,
    @SerialName("policy_revision") val policyRevision: Long,
    val grant: LocalAppGrantAccess,
)
@Serializable
internal data class LocalAppGrantSnapshot(
    @SerialName("peer_device_id") val peerDeviceId: String,
    @SerialName("peer_public_key") val peerPublicKey: String,
    val catalog: RemoteAppCatalog,
    val relations: List<LocalAppRelation>,
)
@Serializable
internal data class LocalAppGrantChange(
    val target: JsonObject,
    @SerialName("expected_policy_revision") val expectedPolicyRevision: Long,
    @SerialName("host_device_id") val hostDeviceId: String,
    @SerialName("catalog_revision") val catalogRevision: String,
    @SerialName("user_id") val userId: String,
    @SerialName("authorization_scope") val authorizationScope: String,
    @SerialName("app_ids") val appIds: List<String>,
    val decision: String,
    @SerialName("valid_until") val validUntil: Long?,
)
@Serializable
private data class LocalAppGrantResult(val target: JsonObject, val revision: Long, val persisted: Boolean, val applied: Boolean, val error: String? = null)

/** The main process uses the existing protected embedded-worker IPC. No Exec fallback. */
internal object AppsApi {
    suspend fun localGrants(deviceId: String, publicKey: String): LocalAppGrantSnapshot {
        val snapshot: LocalAppGrantSnapshot = Core.request("apps.localGrants", buildJsonObject { put("deviceId", deviceId); put("publicKey", publicKey) })
        check(snapshot.peerDeviceId == deviceId && snapshot.peerPublicKey == publicKey) { "Lokaler Appkatalog gehört zu einer abweichenden Gerätebeziehung." }
        return snapshot
    }
    suspend fun setLocalGrant(change: LocalAppGrantChange): Long {
        val result: LocalAppGrantResult = Core.request("apps.setLocalGrant", buildJsonObject { put("change", Core.json.encodeToJsonElement(change)) })
        check(result.target == change.target) { "Antwort gehört zu einer abweichenden Rechtequelle." }
        check(result.persisted && result.applied && result.error == null) {
            "Appfreigabe nicht vollständig übernommen: gespeichert=${result.persisted}, angewendet=${result.applied}, Grund=${result.error ?: "Änderung im Rechtejournal ausstehend"}"
        }
        return result.revision
    }
    private val diskLock = Any()
    private val diskJson = Json { encodeDefaults = true; ignoreUnknownKeys = false }
    private fun endpoint(target: TcpTarget): String = when (target.relation) {
        "direct" -> "share://direct/${target.relationId}"
        "room" -> "share://room/${target.relationId}/${target.deviceId}"
        else -> error("Keine gespeicherte ausgehende Gerätebeziehung.")
    }
    private fun peer(target: TcpTarget): JsonObject = buildJsonObject {
        when (target.relation) {
            "direct" -> put("Direct", buildJsonObject { put("contact_id", target.relationId) })
            "room" -> put("RoomDevice", buildJsonObject { put("room_id", target.relationId); put("device_id", target.deviceId) })
            else -> error("Keine gespeicherte ausgehende Gerätebeziehung.")
        }
    }
    private suspend fun call(method: String, target: TcpTarget, args: JsonObject = buildJsonObject {}): AppsReply {
        check(target.canStart && target.deviceId.isNotBlank() && target.publicKey.isNotBlank()) { "Bestätigte ausgehende Geräteidentität fehlt." }
        val reply: AppsReply = Core.request(method, buildJsonObject {
            put("target", peer(target)); put("waitSeconds", 30)
            put("expectedDeviceId", target.deviceId); put("expectedPublicKey", target.publicKey); put("expectedNodeId", target.nodeId)
            args.forEach { (key, value) -> put(key, value) }
        })
        check(reply.deviceId == target.deviceId && (target.nodeId.isBlank() || reply.nodeId == target.nodeId)) {
            "Authentifizierte Geräteidentität hat sich geändert. Geräteliste erneut laden."
        }
        return reply
    }
    suspend fun catalog(target: TcpTarget): RemoteAppCatalog {
        val result = call("apps.catalog", target).result
        check(result.kind == "catalog") { "Unerwartete Katalogantwort." }
        return requireNotNull(result.catalog).also {
            check(it.version == 1 && it.deviceId == target.deviceId) { "Katalogidentität oder Katalogversion stimmt nicht." }
            check(it.apps.size <= 8192 && it.apps.map { app -> app.appId }.distinct().size == it.apps.size) { "Katalog ist zu groß oder mehrdeutig." }
        }
    }
    fun request(catalog: RemoteAppCatalog, app: HostCatalogApp): AppLaunchRequest {
        check(app.canLaunch && catalog.apps.any { it == app }) { "App ist nicht im startfähigen Katalog enthalten." }
        return AppLaunchRequest(UUID.randomUUID().toString(), catalog.deviceId, app.appId,
            catalog.session.userId, catalog.session.sessionId, catalog.revision)
    }
    suspend fun launch(target: TcpTarget, request: AppLaunchRequest): AppReceiptResult = result(target, request, "apps.launch")
    suspend fun receipt(target: TcpTarget, request: AppLaunchRequest): AppReceiptResult = result(target, request, "apps.receipt")
    private suspend fun result(target: TcpTarget, request: AppLaunchRequest, method: String): AppReceiptResult {
        check(request.targetDeviceId == target.deviceId) { "Gespeicherter Auftrag gehört zu einer abweichenden Geräteidentität." }
        val reply = call(method, target, buildJsonObject { put("request", Core.json.encodeToJsonElement(request)) })
        check(reply.result.kind == if (method == "apps.launch") "launch" else "receipt") { "Unerwartete Startantwort." }
        val receipt = reply.result.receipt
        check(receipt == null || receipt.request == request) { "Startbeleg passt nicht zum vollständigen gespeicherten Auftrag." }
        check(method != "apps.launch" || receipt != null) { "Startantwort enthält keinen Beleg." }
        return AppReceiptResult(receipt, receipt?.let { ConfirmedRemoteAppWindow.from(target, reply.nodeId, it) })
    }
    suspend fun icon(target: TcpTarget, catalog: RemoteAppCatalog, app: HostCatalogApp): Bitmap? = withContext(Dispatchers.IO) {
        val reference = app.icon ?: return@withContext null
        check(reference.appId == app.appId && catalog.apps.any { it == app }) { "Symbol ist nicht an diese Katalog-App gebunden." }
        validate(reference)
        val result = call("apps.icon", target, buildJsonObject {
            put("appId", app.appId); put("icon", Core.json.encodeToJsonElement(reference)); put("catalogRevision", catalog.revision)
        }).result
        check(result.kind == "icon") { "Unerwartete Symbolantwort." }
        val reply = result.reply ?: return@withContext null
        check(reply.icon == reference && reply.pngBase64.length <= 349528) { "Symbolantwort stimmt nicht mit der Referenz überein." }
        val bytes = Base64.decode(reply.pngBase64, Base64.DEFAULT)
        check(bytes.size in 33..262144 && hash(bytes) == reference.sha256) { "Symbolgröße oder SHA-256 stimmt nicht." }
        check(bytes.take(8) == listOf(137, 80, 78, 71, 13, 10, 26, 10).map { it.toByte() }) { "Symbol ist kein PNG." }
        var offset = 8
        while (offset + 12 <= bytes.size) {
            val length = (0..3).fold(0L) { n, i -> (n shl 8) or (bytes[offset + i].toLong() and 255) }
            check(length <= bytes.size - offset - 12) { "Ungültiger PNG-Chunk." }
            val kind = bytes.copyOfRange(offset + 4, offset + 8).toString(Charsets.US_ASCII)
            check(kind != "acTL") { "Animierte Appsymbole werden nicht angenommen." }
            offset += length.toInt() + 12
        }
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        check(bounds.outMimeType == "image/png" && bounds.outWidth == reference.width && bounds.outHeight == reference.height) { "PNG-Abmessungen stimmen nicht." }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.also {
            if (it.width != reference.width || it.height != reference.height) { it.recycle(); error("Dekodierte Symbolgröße stimmt nicht.") }
        } ?: error("PNG konnte nicht dekodiert werden.")
    }
    private fun validate(icon: ScreenIconRef) {
        check(icon.version == 1 && icon.appId.toByteArray().size in 1..1024 && icon.appId.none { it.isISOControl() } &&
            icon.sha256.matches(Regex("[0-9a-f]{64}")) && icon.width in 1..256 && icon.height in 1..256) { "Ungültige Symbolreferenz." }
    }
    private fun hash(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it.toInt() and 255) }
    private fun journal(context: Context, target: TcpTarget): File = File(context.filesDir, "remote-app-launches/" + hash(endpoint(target).toByteArray()))
    suspend fun load(context: Context, target: TcpTarget): SavedRemoteApp? = withContext(Dispatchers.IO) {
        synchronized(diskLock) {
            val file = AtomicFile(File(journal(context, target), "latest.json"))
            val bytes = try { file.openRead().use { it.readBytesBounded() } } catch (_: FileNotFoundException) { return@synchronized null }
            val saved = diskJson.decodeFromString<SavedRemoteApp>(bytes.toString(Charsets.UTF_8))
            check(saved.endpoint == endpoint(target) && saved.publicKey == target.publicKey && saved.request.targetDeviceId == target.deviceId) {
                "Gespeicherter Startauftrag gehört zu einer geänderten Geräteidentität. Er wird nicht erneut gestartet."
            }
            saved
        }
    }
    suspend fun save(context: Context, target: TcpTarget, name: String, request: AppLaunchRequest): SavedRemoteApp = withContext(Dispatchers.IO) {
        synchronized(diskLock) {
            val saved = SavedRemoteApp(endpoint(target), target.publicKey, name, request)
            val bytes = diskJson.encodeToString(saved).toByteArray()
            check(bytes.size <= 32768) { "Startauftrag überschreitet das Speicherlimit." }
            val directory = journal(context, target)
            check(directory.isDirectory || directory.mkdirs()) { "Privates Programmjournal nicht verfügbar; kein Start versandt." }
            // Full immutable bodies remain available even when another explicit
            // launch becomes the latest entry. No filesystem name uses an ID verbatim.
            val immutable = AtomicFile(File(directory, hash(request.requestId.toByteArray()) + ".json"))
            val existing = try { immutable.openRead().use { it.readBytesBounded() } } catch (_: FileNotFoundException) { null }
            if (existing != null) {
                val original = diskJson.decodeFromString<SavedRemoteApp>(existing.toString(Charsets.UTF_8))
                check(original.request == request && original.endpoint == saved.endpoint && original.publicKey == saved.publicKey) {
                    "Request-ID ist bereits an einen abweichenden vollständigen Auftrag gebunden."
                }
            } else write(immutable, bytes)
            write(AtomicFile(File(directory, "latest.json")), bytes)
            saved
        }
    }
    private fun write(file: AtomicFile, bytes: ByteArray) {
        val output = file.startWrite()
        try { output.write(bytes); file.finishWrite(output) } catch (error: Exception) { file.failWrite(output); throw error }
    }
    private fun java.io.InputStream.readBytesBounded(): ByteArray {
        val output = java.io.ByteArrayOutputStream()
        val buffer = ByteArray(4096)
        while (true) { val count = read(buffer); if (count < 0) break; check(output.size() + count <= 32768) { "Programmjournal ist zu groß." }; output.write(buffer, 0, count) }
        return output.toByteArray()
    }
}
