package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// This phone as exec host, api.md §5 (`execProvider`, `execTargets`, `share.setExec`,
// `share.execJobs`, `share.cancelExecJob`); field names exactly as the contract.

/** `ShareStatus.execProvider`: whether commands of other devices can run here, and why not. */
@Serializable
data class ExecProviderInfo(
    val available: Boolean = false,
    val provider: String = "",
    val detail: String = "",
)

/** A device that may be allowed to run commands here: a Direct device or a room member. */
@Serializable
data class ExecTarget(
    val targetKey: String,
    /** `direct` or `room`. */
    val relation: String = EXEC_RELATION_DIRECT,
    val roomId: String? = null,
    val roomName: String? = null,
    val deviceId: String = "",
    val name: String = "",
    val fingerprint: String = "",
    val enabled: Boolean = false,
    /** The file relation the grant builds on is active; otherwise it cannot be enabled. */
    val baseAuthorized: Boolean = false,
    val policyRevision: Long = 0,
)

/**
 * A command in either direction; [state] is the lifecycle in snake_case (`running`, `exited`,
 * `cancelled`, `timed_out`, `revoked`, …), times are seconds since 1970.
 */
@Serializable
data class ExecJob(
    /** `incoming` (runs on this phone) or `outgoing`. */
    val direction: String = EXEC_INCOMING,
    val execId: String,
    val peerDeviceId: String = "",
    val peerName: String = "",
    val program: String = "",
    val state: String = "",
    val startedAt: Long? = null,
    val finishedAt: Long? = null,
    val exitCode: Int? = null,
    val message: String? = null,
)

/** `share.execJobs` */
@Serializable
data class ExecJobs(
    val active: List<ExecJob> = emptyList(),
    val history: List<ExecJob> = emptyList(),
)

const val EXEC_RELATION_DIRECT = "direct"
const val EXEC_RELATION_ROOM = "room"
const val EXEC_INCOMING = "incoming"

/** Suspending wrappers of the exec-host methods; every call throws `CoreException` on errors. */
object ShareExecApi {
    /**
     * Allows or revokes commands of [targetKey]; returns the new policy revision. Succeeds only
     * once the change is stored and applied (`not_found`: the device's identity changed,
     * `unsupported`: no exec provider on this phone).
     */
    suspend fun setExec(targetKey: String, enabled: Boolean): Long = Core.request<Revision>(
        "share.setExec",
        buildJsonObject {
            put("targetKey", targetKey)
            put("enabled", enabled)
        },
    ).revision

    suspend fun jobs(): ExecJobs = Core.request<ExecJobs>("share.execJobs")

    /** Stops [job]; `not_found` once it no longer runs. */
    suspend fun cancel(job: ExecJob) {
        Core.call(
            "share.cancelExecJob",
            buildJsonObject {
                put("direction", job.direction)
                put("execId", job.execId)
                put("peerDeviceId", job.peerDeviceId)
            },
        )
    }

    @Serializable
    private data class Revision(val revision: Long)
}
