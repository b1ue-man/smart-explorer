package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray

// This phone as screen host (plan 2026-10-17, SCR-M8; api.md „screen.host*“): the app reports what
// it can (`screen.hostPrereqs`), captures and encodes the sessions the core starts
// (`screenHostStart`), reports them ready or ended and lists them; field names exactly as the
// contract. Pictures and control messages take the binary JNI path (`NativeBridge.screenPush`,
// `NativeBridge.screenControl`).
//
// This phone as screen viewer (SCR-M9, `screen.devices` … `screen.viewClose`): the devices, the
// probe of one device, its window list and the sessions; progress of a session comes as
// `ScreenViewEvent` (`core/CoreEvents.kt`), its pictures through `NativeBridge.screenPull`.

const val ACCESSIBILITY_ON = "on"
const val ACCESSIBILITY_OFF = "off"

/** „Eingeschränkte Einstellungen“ block the switch (sideloaded app, Android 13+). */
const val ACCESSIBILITY_RESTRICTED = "restricted"

/** What the app reports and the core keeps (`screen.hostPrereqs`). */
@Serializable
data class ScreenPrereqs(
    val accessibility: String = ACCESSIBILITY_OFF,
    val usageAccess: Boolean = false,
    val appWindowCapture: Boolean = false,
)

/** One H.264 encoder of MediaCodec: `cb` or `high`. */
@Serializable
data class ScreenEncoderOffer(val profile: String, val hardware: Boolean)

/** A running app as the viewer's window list shows it. */
@Serializable
data class ScreenAppEntry(val packageName: String, val label: String)

/** One host session (`screen.hostSessions`). */
@Serializable
data class ScreenHostSession(
    val session: Long,
    val source: String = "",
    val still: Boolean = false,
    val viewerName: String = "",
    val viewerDeviceId: String = "",
    /** `consent`, `streaming` or `ended`. */
    val state: String = "",
    val width: Int = 0,
    val height: Int = 0,
    val encoder: String = "",
    val projection: Boolean = false,
)

@Serializable
data class ScreenHostSessions(val sessions: List<ScreenHostSession> = emptyList())

/** Suspending wrappers of `screen.host*`; every call throws `CoreException` on errors. */
object ScreenApi {
    /** Registers the app as host and stores what it reports; `null` fields stay as they are. */
    suspend fun prereqs(
        accessibility: String? = null,
        usageAccess: Boolean? = null,
        appWindowCapture: Boolean? = null,
        display: Pair<Int, Int>? = null,
        encoders: List<ScreenEncoderOffer>? = null,
        apps: List<ScreenAppEntry>? = null,
    ): ScreenPrereqs = Core.request<ScreenPrereqs>(
        "screen.hostPrereqs",
        buildJsonObject {
            accessibility?.let { put("accessibility", it) }
            usageAccess?.let { put("usageAccess", it) }
            appWindowCapture?.let { put("appWindowCapture", it) }
            display?.let { (width, height) ->
                put(
                    "display",
                    buildJsonObject {
                        put("width", width)
                        put("height", height)
                    },
                )
            }
            encoders?.let { list ->
                put(
                    "encoders",
                    JsonArray(
                        list.map {
                            buildJsonObject {
                                put("profile", it.profile)
                                put("hardware", it.hardware)
                            }
                        },
                    ),
                )
            }
            apps?.let { list ->
                put(
                    "apps",
                    JsonArray(
                        list.map {
                            buildJsonObject {
                                put("package", it.packageName)
                                put("label", it.label)
                            }
                        },
                    ),
                )
            }
        },
    )

    suspend fun sessions(): List<ScreenHostSession> = Core.request<ScreenHostSessions>("screen.hostSessions").sessions

    /** Ends [session], or every session when it is `null` (the host user's „Beenden“). */
    suspend fun stop(session: Long?): Int {
        val args: JsonObject = buildJsonObject {
            if (session == null) put("session", "all") else put("session", session)
        }
        val answer = Core.call("screen.hostStop", args) as? JsonObject
        return answer?.get("stopped")?.toString()?.toIntOrNull() ?: 0
    }

    /** The app captures [session] at [width]×[height]; [projection] after the system dialog. */
    suspend fun ready(session: Long, width: Int, height: Int, encoder: String, hardware: Boolean, projection: Boolean) {
        Core.call(
            "screen.hostReady",
            buildJsonObject {
                put("session", session)
                put("width", width)
                put("height", height)
                put("encoder", encoder)
                put("hardware", hardware)
                put("projection", projection)
            },
        )
    }

    /** The app ended [session]: `consent_denied`, `host_locked`, `host_stopped`, `capture_failed`. */
    suspend fun ended(session: Long, reason: String) {
        Core.call(
            "screen.hostEnded",
            buildJsonObject {
                put("session", session)
                put("reason", reason)
            },
        )
    }

    // --- This phone as screen viewer (SCR-M9) ---

    private fun targetArgs(relation: String, relationId: String, deviceId: String?) = buildJsonObject {
        put("relation", relation)
        put("relationId", relationId)
        if (deviceId != null) put("deviceId", deviceId)
    }

    /** The Direct devices and room members; [roomId] limits it to one room. */
    suspend fun devices(roomId: String? = null): List<ScreenDevice> = Core.request<ScreenDevices>(
        "screen.devices",
        buildJsonObject { if (roomId != null) put("roomId", roomId) },
    ).devices

    suspend fun probe(relation: String, relationId: String, deviceId: String?): ScreenProbe =
        Core.request("screen.probe", targetArgs(relation, relationId, deviceId))

    suspend fun windows(relation: String, relationId: String, deviceId: String?): ScreenWindows =
        Core.request("screen.windows", targetArgs(relation, relationId, deviceId))

    /** Opens a session ([mode] `view`, `control` or `preview`); its progress comes as events. */
    suspend fun viewOpen(
        relation: String,
        relationId: String,
        deviceId: String?,
        mode: String,
        source: String?,
        decoders: List<DecoderOffer>,
        displayWidth: Int,
        displayHeight: Int,
        maxFps: Int,
    ): Long {
        val args = buildJsonObject {
            put("relation", relation)
            put("relationId", relationId)
            if (deviceId != null) put("deviceId", deviceId)
            put("mode", mode)
            if (source != null) put("source", source)
            putJsonArray("decoders") {
                decoders.forEach { offer ->
                    add(Core.json.encodeToJsonElement(DecoderOffer.serializer(), offer))
                }
            }
            put("displayWidth", displayWidth)
            put("displayHeight", displayHeight)
            put("maxFps", maxFps)
        }
        val answer = Core.call("screen.viewOpen", args) as? JsonObject
        return answer?.get("session")?.toString()?.toLongOrNull() ?: 0L
    }

    /** `preview` or `full`; the other fields keep the host's choice when `null`. */
    suspend fun viewSetMode(
        session: Long,
        mode: String,
        source: String? = null,
        width: Int? = null,
        height: Int? = null,
        fps: Int? = null,
        sharpness: Boolean = false,
        lossless: Boolean = false,
    ) {
        Core.call(
            "screen.viewSetMode",
            buildJsonObject {
                put("session", session)
                put("mode", mode)
                if (source != null) put("source", source)
                if (width != null && height != null) {
                    put("width", width)
                    put("height", height)
                }
                if (fps != null) put("fps", fps)
                put("sharpness", sharpness)
                put("lossless", lossless)
            },
        )
    }

    /** Sends input events (the serde form of the core's `InputEvent`); the answer is the count. */
    suspend fun viewInput(session: Long, events: List<JsonObject>): Int {
        val args = buildJsonObject {
            put("session", session)
            put("events", JsonArray(events))
        }
        val answer = Core.call("screen.viewInput", args) as? JsonObject
        return answer?.get("sent")?.toString()?.toIntOrNull() ?: 0
    }

    /** `take` or `release` the control of the device. */
    suspend fun viewControl(session: Long, action: String) {
        Core.call("screen.viewControl", buildJsonObject { put("session", session); put("action", action) })
    }

    /** Asks for the window list; it comes as a `windows` event. */
    suspend fun viewListWindows(session: Long) {
        Core.call("screen.viewListWindows", buildJsonObject { put("session", session) })
    }

    /** The decoder failed: one key frame request. */
    suspend fun viewKeyframe(session: Long) {
        Core.call("screen.viewKeyframe", buildJsonObject { put("session", session) })
    }

    /** Ends the session; closing twice is fine. */
    suspend fun viewClose(session: Long) {
        Core.call("screen.viewClose", buildJsonObject { put("session", session) })
    }
}

/** One device the viewer can open (`screen.devices`); `null` fields are not known yet. */
@Serializable
data class ScreenDevice(
    /** `direct` or `room`. */
    val relation: String,
    val relationId: String,
    val deviceId: String,
    val name: String = "",
    val roomId: String? = null,
    val roomName: String? = null,
    val online: Boolean = false,
    val supported: Boolean? = null,
    /** `off`, `ask` or `allow` after a probe. */
    val view: String? = null,
    val control: String? = null,
)

@Serializable
data class ScreenDevices(val devices: List<ScreenDevice> = emptyList())

@Serializable
data class ScreenMonitor(
    val id: String,
    val name: String = "",
    val x: Int = 0,
    val y: Int = 0,
    val width: Int = 0,
    val height: Int = 0,
    val primary: Boolean = false,
)

/** What the host can inject (`InputCapabilities`): [actions] are system actions, [toolbarKeys] HID usages. */
@Serializable
data class ScreenInputCaps(
    val pointer: Boolean = false,
    val keyboard: Boolean = false,
    val text: Boolean = false,
    val touch: Boolean = false,
    val actions: List<String> = emptyList(),
    val toolbarKeys: List<Int> = emptyList(),
)

/** `screen.probe`: rights and abilities of a device. */
@Serializable
data class ScreenProbe(
    val device: String = "",
    val supported: Boolean = false,
    val platform: String? = null,
    val view: String = "off",
    val control: String = "off",
    val monitors: List<ScreenMonitor> = emptyList(),
    val input: ScreenInputCaps = ScreenInputCaps(),
    val limits: List<String> = emptyList(),
)

@Serializable
data class ScreenWindow(
    val id: String,
    val title: String = "",
    val app: String = "",
    val minimized: Boolean = false,
    val focused: Boolean = false,
)

@Serializable
data class ScreenApp(val appId: String, val name: String = "")

/** `screen.windows` and the `windows` event. */
@Serializable
data class ScreenWindows(
    val windows: List<ScreenWindow> = emptyList(),
    val apps: List<ScreenApp> = emptyList(),
    val listSource: String = "",
    val limits: List<String> = emptyList(),
)

/** One decoder of this phone offered to the host (`decoders` of `screen.viewOpen`). */
@Serializable
data class DecoderOffer(
    /** `h264` or `tiles`. */
    val codec: String,
    /** `cb` or `high` for H.264. */
    val profile: String? = null,
    val hardware: Boolean = false,
    val maxWidth: Int? = null,
    val maxHeight: Int? = null,
)
