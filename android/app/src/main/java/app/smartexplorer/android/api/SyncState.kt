package app.smartexplorer.android.api

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject

@Serializable
data class SyncState(
    val lastAttemptMs: Long? = null,
    val lastSuccessMs: Long? = null,
    val lastRunner: String? = null,
    val lastCause: String? = null,
    val consecutiveFailures: Int = 0,
    val lastError: SyncFailure? = null,
    val retryAtMs: Long? = null,
    val blocked: SyncBlock? = null,
    val problem: String? = null,
    val running: SyncRunning? = null,
    val loadError: String? = null,
    val watch: SyncWatch? = null,
    val lastVerifyMs: Long? = null,
    val verifyCursor: String? = null,
    val pendingTrigger: SyncPendingTrigger? = null,
)

@Serializable
data class SyncPendingTrigger(val kind: String = "other", val sinceMs: Long = 0)

@Serializable
data class SyncMergeFailure(val partial: Boolean = false, val reload: Boolean = false, val retry: Boolean = false)

@Serializable
data class SyncMergeRows(val rows: List<MergeRow> = emptyList(), val pending: SyncPendingMerge? = null)

@Serializable
data class SyncPendingMerge(
    val kind: String = "",
    val keepA: Boolean? = null,
    val confirmedA: Boolean = false,
    val confirmedB: Boolean = false,
    val preview: String? = null,
    val previewTruncated: Boolean = false,
)

@Serializable
data class SyncFailure(val kind: String = "other", val message: String = "")

@Serializable
data class SyncBlock(
    val kind: JsonObject,
    val detail: String = "",
    val sinceMs: Long = 0,
    val confirmed: Boolean = false,
)

@Serializable
data class SyncRunning(
    val runner: String = "other",
    val cause: String = "other",
    val startedMs: Long = 0,
    val aliveMs: Long = 0,
    val stalledSinceMs: Long? = null,
)

@Serializable
data class SyncWatch(val detection: SyncDetection? = null, val note: String? = null)

@Serializable
data class SyncDetection(val mode: String = "other", @SerialName("poll_secs") val pollSecs: Long = 0)

@Serializable
data class SyncVersion(
    val token: String,
    val path: String = "",
    val side: String? = null,
    val runId: String = "",
    val preservedMs: Long = 0,
    val size: Long = 0,
    val reason: String? = null,
    val store: String = "",
)

@Serializable
data class SyncVersions(val items: List<SyncVersion> = emptyList())

@Serializable
data class LastCatchUpResult(val finishedMs: Long = 0, val ran: Int = 0, val succeeded: Int = 0,
    val failed: Int = 0, val message: String = "")
