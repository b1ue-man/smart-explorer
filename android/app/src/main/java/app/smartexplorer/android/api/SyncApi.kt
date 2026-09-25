package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.TaskInfo
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.mapNotNull
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.encodeToJsonElement
import kotlinx.serialization.json.put

// Sync jobs, conflicts and the background worker (api.md §4.7), plus the few `sys.*`/`task.*`
// calls the background side needs. Field names exactly as api.md (camelCase); defaults keep
// decoding tolerant and `Core.json` encodes them, so requests always carry every field.

/** One `{value, label}` pair of `sync.options`; `value` is the Rust enum's `as_str()`. */
@Serializable
data class SyncChoice(
    val value: String,
    val label: String = "",
)

/** `Job.calendar`; `weekday` is the desktop bitmask (bit0 = Mo … bit6 = So), `monthday` 1–31. */
@Serializable
data class SyncCalendar(
    val kind: String = "",
    val minuteOfDay: Int = 0,
    val weekday: Int = 0,
    val monthday: Int = 0,
)

@Serializable
data class SyncLastResult(
    val timeMs: Long = 0,
    val aToB: Int = 0,
    val bToA: Int = 0,
    val deleted: Int = 0,
    val conflicts: Int = 0,
    val errors: Int = 0,
    val note: String = "",
)

/** api.md `Job`; `schedule`, `lastResult` and `runningTask` are output only. */
@Serializable
data class SyncJob(
    val id: String = "",
    val name: String = "",
    val source: String = "",
    val target: String = "",
    val direction: String = DIRECTION_BOTH,
    val conflict: String = "",
    val deletePolicy: String = "",
    val compare: String = "",
    val versioning: String = "",
    val retainDays: Int = 0,
    val trigger: String = TRIGGER_MANUAL,
    val intervalMin: Int = 60,
    val calendar: SyncCalendar? = null,
    val rtDebounceSecs: Int = 0,
    val includeHidden: Boolean = false,
    val ignore: List<String> = emptyList(),
    val enabled: Boolean = true,
    val runBefore: String = "",
    val runAfter: String = "",
    val lastRun: Long = 0,
    val activeFromMin: Int = 0,
    val activeToMin: Int = 0,
    val catchUp: Boolean = false,
    val moveFiles: Boolean = false,
    val maxDelete: Int = 0,
    val maxDeletePct: Int = 0,
    val useRecycleBin: Boolean = false,
    val lastResult: SyncLastResult? = null,
    val schedule: String = "",
    val runningTask: String? = null,
) {
    companion object {
        const val DIRECTION_BOTH = "both"
        const val DIRECTION_A_TO_B = "a2b"
        const val DIRECTION_B_TO_A = "b2a"
        const val TRIGGER_MANUAL = "manual"
        const val TRIGGER_INTERVAL = "interval"
        const val TRIGGER_CALENDAR = "calendar"
        const val TRIGGER_REALTIME = "realtime"
        const val TRIGGER_STARTUP = "onstartup"
        const val TRIGGER_CONNECT = "onconnect"
        const val DELETE_NONE = "nodelete"
    }
}

/** `sync.options`: every choice list with display labels, plus the defaults of a new job. */
@Serializable
data class SyncOptions(
    val directions: List<SyncChoice> = emptyList(),
    val conflicts: List<SyncChoice> = emptyList(),
    val deletePolicies: List<SyncChoice> = emptyList(),
    val compares: List<SyncChoice> = emptyList(),
    val versionings: List<SyncChoice> = emptyList(),
    val triggers: List<SyncChoice> = emptyList(),
    val calendarKinds: List<SyncChoice> = emptyList(),
    val defaults: SyncJob = SyncJob(),
)

@Serializable
data class ConflictSide(
    val exists: Boolean = false,
    val size: Long = 0,
    val mtimeMs: Long = 0,
)

/** One open conflict; [cid] is passed back to the core unchanged (its JSON type is the core's). */
@Serializable
data class SyncConflict(
    val cid: JsonPrimitive,
    val path: String = "",
    val a: ConflictSide? = null,
    val b: ConflictSide? = null,
    val text: Boolean = false,
) {
    /** Stable key for lists (the JSON form of [cid]). */
    val key: String get() = cid.toString()
}

@Serializable
data class SyncConflicts(
    val available: Boolean = false,
    val items: List<SyncConflict> = emptyList(),
)

/** One line pair of the line merge (`linemerge::rows`); `null` = the line is missing on that side. */
@Serializable
data class MergeRow(
    val a: String? = null,
    val b: String? = null,
    val equal: Boolean = false,
    val takeA: Boolean = false,
    val takeB: Boolean = false,
)

@Serializable
data class MergeChoice(
    val takeA: Boolean,
    val takeB: Boolean,
)

/** `result` of a `sync.run` task. */
@Serializable
data class SyncRunResult(
    val summary: String = "",
    val aToB: Int = 0,
    val bToA: Int = 0,
    val deleted: Int = 0,
    val conflicts: Int = 0,
    val errors: Int = 0,
    val omitted: String? = null,
)

/** `result` of a `sync.mirror` task. */
@Serializable
data class MirrorResult(
    val copied: Long = 0,
    val skipped: Long = 0,
    val errors: Long = 0,
    val omitted: String? = null,
)

/** A job the supervisor did not admit to a catch-up run, with its reason. */
@Serializable
data class CatchUpSkip(
    val jobId: String = "",
    val jobName: String = "",
    val reason: String = "",
)

/** `result` of a `bg.catchUp` task (`kind == "catchup"`). */
@Serializable
data class CatchUpResult(
    val admitted: Int = 0,
    val skipped: List<CatchUpSkip> = emptyList(),
)

/** `bg.status`. */
@Serializable
data class BgStatus(
    val syncEnabled: Boolean = false,
    val daemonRunning: Boolean = false,
    val heartbeatAgeSecs: Long? = null,
    val paused: Boolean = false,
    /** `null` while paused = paused without end. */
    val pausedUntilMs: Long? = null,
    val autopauseBattery: Boolean = false,
    val autopauseMetered: Boolean = false,
    val cadenceSecs: Int = 0,
    val catchUpRunning: Boolean = false,
    val lastCatchUpMs: Long? = null,
    /** Display name of the job the daemon runs right now. */
    val activeJob: String? = null,
)

/** `sys.hostState` arguments. */
data class HostStateArgs(
    val powerSave: Boolean,
    val metered: Boolean,
    val wifi: Boolean,
    val charging: Boolean,
    val foreground: Boolean,
)

object SyncApi {
    /** Task kind of `bg.catchUp` runs; they never keep the task service alive. */
    const val KIND_CATCH_UP = "catchup"

    suspend fun options(): SyncOptions = Core.request<SyncOptions>("sync.options")

    suspend fun jobs(): List<SyncJob> = Core.request<List<SyncJob>>("sync.jobs")

    /** Desktop validation; field name → message (empty = valid). */
    suspend fun validate(job: SyncJob): Map<String, String> =
        Core.request<Validation>("sync.validate", jobArgs(job)).errors

    /** Saves [job] (empty id = new) and returns the stored job. */
    suspend fun save(job: SyncJob): SyncJob = Core.request<SyncJob>("sync.save", jobArgs(job))

    suspend fun delete(id: String) {
        Core.call("sync.delete", buildJsonObject { put("id", id) })
    }

    suspend fun setEnabled(id: String, enabled: Boolean): SyncJob =
        Core.request<SyncJob>("sync.setEnabled", buildJsonObject {
            put("id", id)
            put("enabled", enabled)
        })

    /** Starts the job now; returns the task id. */
    suspend fun run(id: String): String = taskOf("sync.run", buildJsonObject { put("id", id) })

    /** One-way copy of new and changed files; nothing is deleted in [target]. */
    suspend fun mirror(source: String, target: String): String =
        taskOf("sync.mirror", buildJsonObject {
            put("source", source)
            put("target", target)
        })

    suspend fun conflicts(id: String): SyncConflicts =
        Core.request<SyncConflicts>("sync.conflicts", buildJsonObject { put("id", id) })

    /** Dry run that fills the conflict context of the job. */
    suspend fun checkConflicts(id: String): String = taskOf("sync.checkConflicts", buildJsonObject { put("id", id) })

    /** [choice] is `"a"` or `"b"`. */
    suspend fun resolve(id: String, cid: JsonPrimitive, choice: String): String =
        taskOf("sync.resolve", buildJsonObject {
            put("id", id)
            put("cid", cid)
            put("choice", choice)
        })

    suspend fun skip(id: String, cid: JsonPrimitive) {
        Core.call("sync.skip", conflictArgs(id, cid))
    }

    /** Stores a changed baseline; throws when that fails (the caller offers a retry). */
    suspend fun finishConflicts(id: String) {
        Core.call("sync.finishConflicts", buildJsonObject { put("id", id) })
    }

    suspend fun mergeRows(id: String, cid: JsonPrimitive): List<MergeRow> =
        Core.request<MergeRows>("sync.mergeRows", conflictArgs(id, cid)).rows

    suspend fun mergeApply(id: String, cid: JsonPrimitive, rows: List<MergeChoice>): String =
        taskOf("sync.mergeApply", buildJsonObject {
            put("id", id)
            put("cid", cid)
            put("rows", Core.json.encodeToJsonElement(rows))
        })

    suspend fun mergeKeepBoth(id: String, cid: JsonPrimitive): String = taskOf("sync.mergeKeepBoth", conflictArgs(id, cid))

    /** Starts the embedded daemon if needed; waits up to 10 s in the core. */
    suspend fun ensureDaemon(): Boolean = Core.request<Running>("bg.ensureDaemon").running

    suspend fun status(): BgStatus = Core.request<BgStatus>("bg.status")

    suspend fun setSyncEnabled(enabled: Boolean) {
        Core.call("bg.setSyncEnabled", buildJsonObject { put("enabled", enabled) })
    }

    /** [seconds] = −1 pauses without end. */
    suspend fun pause(seconds: Long) {
        Core.call("bg.pause", buildJsonObject { put("seconds", seconds) })
    }

    suspend fun resume() {
        Core.call("bg.resume")
    }

    suspend fun setAutopause(battery: Boolean, metered: Boolean) {
        Core.call("bg.setAutopause", buildJsonObject {
            put("battery", battery)
            put("metered", metered)
        })
    }

    suspend fun log(maxBytes: Int): String = Core.request<LogText>("bg.log", buildJsonObject { put("maxBytes", maxBytes) }).text

    /** Catch-up run of the daemon; returns the task id (`kind == "catchup"`). */
    suspend fun catchUp(): String = taskOf("bg.catchUp", JsonObject(emptyMap()))

    suspend fun hostState(state: HostStateArgs) {
        Core.call("sys.hostState", buildJsonObject {
            put("powerSave", state.powerSave)
            put("metered", state.metered)
            put("wifi", state.wifi)
            put("charging", state.charging)
            put("foreground", state.foreground)
        })
    }

    suspend fun task(id: String): TaskInfo = Core.request<TaskInfo>("task.get", buildJsonObject { put("id", id) })

    suspend fun cancelTask(id: String) {
        Core.call("task.cancel", buildJsonObject { put("id", id) })
    }

    suspend fun cancelAllTasks() {
        Core.call("task.cancelAll")
    }

    /**
     * Waits until task [id] has ended and returns its final state. Task events keep [Core.tasks]
     * current; a `task.get` every [pollMs] covers a missed event.
     */
    suspend fun awaitTask(id: String, pollMs: Long = 15_000): TaskInfo {
        while (true) {
            val ended = withTimeoutOrNull(pollMs) {
                Core.tasks.mapNotNull { list -> list.firstOrNull { it.id == id } }.first { !it.isActive }
            }
            if (ended != null) return ended
            val fresh = task(id)
            if (!fresh.isActive) return fresh
        }
    }

    private suspend fun taskOf(method: String, args: JsonObject): String = Core.request<TaskRef>(method, args).taskId

    private fun jobArgs(job: SyncJob): JsonObject = buildJsonObject { put("job", Core.json.encodeToJsonElement(job)) }

    private fun conflictArgs(id: String, cid: JsonPrimitive): JsonObject = buildJsonObject {
        put("id", id)
        put("cid", cid)
    }

    /** Decoded `result` of a finished task, or `null` when it is missing or has another shape. */
    inline fun <reified T> TaskInfo.resultAs(): T? {
        val element: JsonElement = result ?: return null
        return try {
            Core.json.decodeFromJsonElement<T>(element)
        } catch (e: IllegalArgumentException) {
            // SerializationException is an IllegalArgumentException.
            null
        }
    }

    /** First error of a task as display text (message, else the first path error). */
    fun TaskInfo.failureText(): String =
        message?.takeIf { it.isNotBlank() }
            ?: errors.firstOrNull()?.let { if (it.path.isBlank()) it.message else "${it.path}: ${it.message}" }
            ?: when (state) {
                "canceled" -> "Abgebrochen"
                else -> "Fehlgeschlagen"
            }

    /** Message of a [CoreException] (or any exception) for snackbars and logs. */
    fun Throwable.displayText(): String =
        message?.takeIf { it.isNotBlank() } ?: (this as? CoreException)?.kind ?: javaClass.simpleName

    // Response shapes used only here (nested, so equally named helpers of other api files never clash).

    @Serializable
    private data class TaskRef(val taskId: String)

    @Serializable
    private data class Running(val running: Boolean = false)

    @Serializable
    private data class LogText(val text: String = "")

    @Serializable
    private data class MergeRows(val rows: List<MergeRow> = emptyList())

    @Serializable
    private data class Validation(val errors: Map<String, String> = emptyMap())
}
