package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// This phone as screen host (plan 2026-10-17, SCR-M8; api.md „screen.host*“): the app reports what
// it can (`screen.hostPrereqs`), captures and encodes the sessions the core starts
// (`screenHostStart`), reports them ready or ended and lists them; field names exactly as the
// contract. Pictures and control messages take the binary JNI path (`NativeBridge.screenPush`,
// `NativeBridge.screenControl`).

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
}
