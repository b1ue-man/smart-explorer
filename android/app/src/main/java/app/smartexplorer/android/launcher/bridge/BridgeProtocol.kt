package app.smartexplorer.android.launcher.bridge

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.put

/**
 * The Messenger protocol between the launcher process `:launcher` and `LauncherBridgeService` in the main process
 * (DSK-M13, U40-4). Both processes read this file; it imports nothing from the main process (U35-11), only the
 * JSON library. The messages are valid only between the two processes of one app: the service drops every message
 * whose `sendingUid` is not its own.
 */
object BridgeProtocol {
    const val VERSION = 1

    /** Client → service; `replyTo` is the client's messenger, `arg1` is [VERSION]. */
    const val SUBSCRIBE = 1

    /** Service → client; the bundle holds [KEY_JSON] with the text of [DevicesJson]. */
    const val DEVICES = 2

    /** Client → service. */
    const val UNSUBSCRIBE = 3

    /** Client → service; the bundle holds [KEY_ACTION], `arg2` is the request number, `replyTo` is set. */
    const val GLOBAL_ACTION = 4

    /** Service → client; `arg2` is the request number, the bundle holds [KEY_JSON] with [ResultJson]. */
    const val RESULT = 5

    const val KEY_JSON = "json"
    const val KEY_ACTION = "action"

    const val ACTION_NOTIFICATIONS = "notifications"
    const val ACTION_LOCK_SCREEN = "lockScreen"

    const val REASON_ACCESSIBILITY_OFF = "accessibility_off"
    const val REASON_UNSUPPORTED = "unsupported"

    const val ERROR_VERSION = "Version nicht unterstützt"
    const val ERROR_TOO_LARGE = "Zu viele Einträge für die Übertragung"

    /** The service class as a string, so the launcher process loads no main-process class to bind it. */
    const val SERVICE_CLASS = "app.smartexplorer.android.service.LauncherBridgeService"

    /** `<filesDir>/launcher/se-cache.json`: the last `DEVICES`, without the token (U40-8). */
    const val SE_CACHE_FILE = "se-cache.json"

    /** `<filesDir>/widget/devices-cache.json`: rows and token for the widget in the launcher process (U40-16). */
    const val WIDGET_DIR = "widget"
    const val WIDGET_CACHE_FILE = "devices-cache.json"

    // Copies of the constants of `AppNav`, so the widget code needs no `AppNav` (U40-16); a JVM test compares them.
    const val NAV_ACTION = "app.smartexplorer.android.action.NAVIGATE"
    const val NAV_EXTRA_KIND = "app.smartexplorer.android.extra.NAV_KIND"
    const val NAV_EXTRA_VALUE = "app.smartexplorer.android.extra.NAV_VALUE"
    const val NAV_EXTRA_TOKEN = "app.smartexplorer.android.extra.NAV_TOKEN"
}

/** The answer to `GLOBAL_ACTION`: `{"ok":true|false,"reason":null|"accessibility_off"|"unsupported"}`. */
data class GlobalResult(val ok: Boolean, val reason: String? = null)

object ResultJson {
    fun encode(result: GlobalResult): String = buildJsonObject {
        put("ok", result.ok)
        put("reason", result.reason)
    }.toString()

    /** `null` for text that is no object. */
    fun parse(text: String): GlobalResult? {
        val root = try {
            Json.parseToJsonElement(text) as? JsonObject
        } catch (_: IllegalArgumentException) {
            null
        } ?: return null
        val reason = (root["reason"] as? JsonPrimitive)?.takeIf { it !is JsonNull }?.contentOrNull
        return GlobalResult((root["ok"] as? JsonPrimitive)?.booleanOrNull ?: false, reason)
    }
}

/**
 * The status codes of `device_view::status_code` for room members, whose `share.status` carries only the label
 * (U40-5). A device of the list carries the code itself.
 */
object StatusCodes {
    const val UNKNOWN = "unknown"
    const val OFFLINE = "offline"
    const val WAITING = "waiting"
    const val WAITING_FOR_ACCESS = "waitingForAccess"
    const val AVAILABLE = "available"
    const val CONNECTING = "connecting"
    const val CONNECTED = "connected"
    const val CONNECTED_DIRECT = "connectedDirect"
    const val CONNECTED_RELAY = "connectedRelay"
    const val FAILED = "failed"
    const val IDENTITY_CONFLICT = "identityConflict"

    private const val FAILED_PREFIX = "Fehler: "

    /** The code of a label of `ShareStatus::label`; any other text is [UNKNOWN]. */
    fun fromLabel(label: String): String = when {
        label == "Offline" -> OFFLINE
        label == "Wartet" -> WAITING
        label == "Warte auf Freigabe" -> WAITING_FOR_ACCESS
        label == "Online" -> AVAILABLE
        label == "Verbinde" -> CONNECTING
        label == "Verbunden" -> CONNECTED
        label == "Direkt verbunden" -> CONNECTED_DIRECT
        label == "Relay verbunden" -> CONNECTED_RELAY
        label.startsWith(FAILED_PREFIX) -> FAILED
        label == "Identitaetskonflikt" -> IDENTITY_CONFLICT
        else -> UNKNOWN
    }

    /** `statusText` of a room member: the label, but for a failure only the text after „Fehler: “ (U40-5). */
    fun statusTextOf(label: String): String = if (label.startsWith(FAILED_PREFIX)) label.removePrefix(FAILED_PREFIX) else label

    /** The reason of a failure for the description of a tile; works for both forms of `statusText`. */
    fun errorDetail(statusText: String): String = statusText.removePrefix(FAILED_PREFIX)

    /** Same rule as `device_view::reachable`: the peer answers or a session runs. */
    fun isOnline(code: String): Boolean = code == AVAILABLE || code == CONNECTED || code == CONNECTED_DIRECT || code == CONNECTED_RELAY
}
