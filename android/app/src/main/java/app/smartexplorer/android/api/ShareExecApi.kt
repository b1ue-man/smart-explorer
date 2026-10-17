package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// This phone as exec host, api.md §5 (`execProvider`, `execTargets`, `share.setExec`,
// `share.execJobs`, `share.cancelExecJob`, `share.execLog`); field names exactly as the contract.

/** `ShareStatus.execProvider`: whether commands of other devices can run here, and why not. */
@Serializable
data class ExecProviderInfo(
    val available: Boolean = false,
    val provider: String = "",
    val detail: String = "",
    /** The app's own account: the Standardnutzer every command here runs as. */
    val userLabel: String = "",
    /** Always `false` on Android: commands cannot run as another account. */
    val runAsSupported: Boolean = false,
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
    /** `standard` (the app's account) or another account name (not possible on Android). */
    val runAs: String = EXEC_RUN_AS_STANDARD,
    /** Empty: the full shell; otherwise the only programs the device may start, without shell or terminal. */
    val programs: List<String> = emptyList(),
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

/**
 * One entry of this phone's Exec audit log (`share.execLog`), newest first; every text field is
 * cut and cleaned by the core. [event] `auth_rejected`, `penalty_refused`, `start_refused`,
 * `started`, `finished`, an access event (`approval_requested`, `approval_granted`,
 * `approval_denied`, `approval_cancelled`, `approval_unavailable`, `grant_expired`,
 * `helper_refused`) or `unreadable`.
 */
@Serializable
data class ExecAuditEntry(
    val at: String = "",
    val event: String = "",
    val title: String = "",
    val device: String = "",
    val deviceId: String = "",
    val detail: String = "",
    /** `exec`, or `screen` for screen sessions; empty for an unreadable line. */
    val kind: String = "",
    /** The account a start ran as; empty when the entry names none. */
    val account: String = "",
    /**
     * How the start was confirmed: `not_required`, `once`, `always`, `denied`, `cancelled` or
     * `unavailable`; empty when the entry has none.
     */
    val approval: String = "",
)

/** `share.execLog` */
@Serializable
data class ExecAuditLog(
    val entries: List<ExecAuditEntry> = emptyList(),
    val max: Int = 0,
)

const val EXEC_RUN_AS_STANDARD = "standard"
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
    suspend fun setExec(targetKey: String, enabled: Boolean): Long = setExec(targetKey, enabled, programs = null)

    /**
     * Like [setExec]; [programs] (when not `null`) replaces the program list in the same step
     * (empty = full shell). Entries are bare names or absolute paths; `invalid` names the refused one.
     * Changing the list of an allowed device ends its running commands.
     */
    suspend fun setExec(targetKey: String, enabled: Boolean, programs: List<String>?): Long = Core.request<Revision>(
        "share.setExec",
        buildJsonObject {
            put("targetKey", targetKey)
            put("enabled", enabled)
            programs?.let { list -> put("programs", JsonArray(list.map { JsonPrimitive(it) })) }
        },
    ).revision

    /** The newest [lines] entries of this phone's audit log (the core caps at [ExecAuditLog.max]). */
    suspend fun log(lines: Int): ExecAuditLog =
        Core.request<ExecAuditLog>("share.execLog", buildJsonObject { put("lines", lines) })

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
