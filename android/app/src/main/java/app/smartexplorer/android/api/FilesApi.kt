package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Crumb
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.core.FilterSpec
import app.smartexplorer.android.core.Root
import app.smartexplorer.android.core.SortSpec
import app.smartexplorer.android.core.TaskInfo
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// Answers of api.md §4.2–§4.5 (camelCase, field names exactly as the contract). Shared types
// (Entry, Crumb, Root, FilterSpec, SortSpec, TaskInfo) come from core/.

/** `loc.roots` */
@Serializable
data class Roots(
    val storage: List<Root> = emptyList(),
    val favorites: List<Root> = emptyList(),
    val recent: List<Root> = emptyList(),
    val connections: List<Root> = emptyList(),
    val gdrive: Root? = null,
    val devices: List<Root> = emptyList(),
    val rooms: List<Root> = emptyList(),
    val trash: Root? = null,
)

@Serializable
data class FavoriteState(val favorite: Boolean)

/** `fs.list` */
@Serializable
data class Listing(
    val location: String,
    val title: String = "",
    val crumbs: List<Crumb> = emptyList(),
    val parent: String? = null,
    /** `local|sftp|ftp|ftps|webdav|gdrive|share|zip|trash` */
    val backend: String = "local",
    val readOnly: Boolean = false,
    val canTrash: Boolean = false,
    val entries: List<Entry> = emptyList(),
    val totalBytes: Long = 0,
) {
    val isLocal: Boolean
        get() = backend == "local"
}

/** `fs.checkName` */
@Serializable
data class NameCheck(val problem: String? = null, val exists: Boolean = false)

/** `fs.conflicts` */
@Serializable
data class ConflictCheck(val names: List<String> = emptyList(), val choosable: Boolean = false)

/** Every method that starts a task answers `{taskId}`. */
@Serializable
data class TaskRef(val taskId: String)

/** `fs.open` */
@Serializable
data class OpenTarget(val localPath: String, val mime: String? = null)

/** Result of the `fs.fetch` task. */
@Serializable
data class FetchResult(val localPath: String, val mime: String? = null, val editId: String = "")

/** Result of the `fs.materialize` task. */
@Serializable
data class MaterializeResult(val paths: List<String> = emptyList())

/** Result of the `fs.properties` task. */
@Serializable
data class PropertiesResult(
    val items: Long = 0,
    val files: Long = 0,
    val dirs: Long = 0,
    val bytes: Long = 0,
    val mtimeMs: Long? = null,
    val btimeMs: Long? = null,
    val location: String? = null,
)

/** Result of a failed `fs.uploadEdit` task when the remote file changed since opening. */
@Serializable
data class UploadConflict(val conflict: Boolean = false)

/** `fs.edits` */
@Serializable
data class EditInfo(
    val editId: String,
    val name: String,
    val location: String,
    val localPath: String,
    val modified: Boolean = false,
)

/** One shared content for `fs.import`; [fd] was detached from its ParcelFileDescriptor. */
@Serializable
data class ImportFile(val fd: Int, val name: String, val size: Long? = null)

/** `scan.validate` */
@Serializable
data class FilterCheck(val error: String? = null)

/** `scan.view` window. */
@Serializable
data class ScanView(
    val revision: Long = 0,
    val unchanged: Boolean = false,
    val entries: List<Entry> = emptyList(),
    val visibleTotal: Int = 0,
    val matches: Long = 0,
    val scanned: Long = 0,
    val truncated: Boolean = false,
    val issues: Int = 0,
)

/** `scan.issues`, `sys.*` style text answers. */
@Serializable
data class TextAnswer(val text: String = "")

/** `index.status` */
@Serializable
data class IndexStatus(
    /** `none|building|ready` */
    val state: String = "none",
    val count: Int = 0,
)

/** One `index.search` hit. */
@Serializable
data class FolderHit(val name: String, val path: String, val location: String, val score: Int = 0)

/** One `trash.list` entry. */
@Serializable
data class TrashItem(
    val id: String,
    val name: String,
    val originalLocation: String,
    val deletedMs: Long = 0,
    val size: Long = 0,
    val isDir: Boolean = false,
)

/** `trash.restore` */
@Serializable
data class RestoreResult(val restored: Int = 0, val renamed: Int = 0)

/** `trash.purge` */
@Serializable
data class PurgeResult(val removed: Int = 0)

/** Suspending wrappers for api.md §4.2–§4.5 plus the task calls of §3; all throw [CoreException]. */
object FilesApi {
    /** Largest `scan.view` window the core accepts. */
    const val MAX_WINDOW = 500

    /** How often [awaitTask] double-checks with `task.get` in case a task event was lost. */
    private const val TASK_RECHECK_MS = 3_000L

    // ---- loc.* ----

    suspend fun roots(): Roots = Core.request<Roots>("loc.roots")

    suspend fun toggleFavorite(location: String): Boolean =
        Core.request<FavoriteState>("loc.toggleFavorite", buildJsonObject { put("location", location) }).favorite

    suspend fun isFavorite(location: String): Boolean =
        Core.request<FavoriteState>("loc.isFavorite", buildJsonObject { put("location", location) }).favorite

    // ---- fs.* ----

    suspend fun list(location: String, showHidden: Boolean, filter: FilterSpec?, sort: SortSpec): Listing =
        Core.request<Listing>(
            "fs.list",
            buildJsonObject {
                put("location", location)
                put("showHidden", showHidden)
                put("filter", filterJson(filter))
                put("sort", sortJson(sort))
            },
        )

    suspend fun stat(location: String): Entry =
        Core.request<Entry>("fs.stat", buildJsonObject { put("location", location) })

    suspend fun checkName(parent: String, name: String): NameCheck =
        Core.request<NameCheck>("fs.checkName", nameArgs(parent, name))

    suspend fun mkdir(parent: String, name: String): Entry = Core.request<Entry>("fs.mkdir", nameArgs(parent, name))

    suspend fun newFile(parent: String, name: String): Entry = Core.request<Entry>("fs.newFile", nameArgs(parent, name))

    suspend fun rename(location: String, newName: String): Entry =
        Core.request<Entry>(
            "fs.rename",
            buildJsonObject {
                put("location", location)
                put("newName", newName)
            },
        )

    suspend fun conflicts(sources: List<String>, targetDir: String): ConflictCheck =
        Core.request<ConflictCheck>(
            "fs.conflicts",
            buildJsonObject {
                put("sources", strings(sources))
                put("targetDir", targetDir)
            },
        )

    /**
     * [mode] `copy|move`, [conflict] `skip|replace|keepBoth` (only local→local). With [filter] and
     * [baseDir] only matching files are copied, with paths relative to [baseDir] (desktop rule).
     */
    suspend fun transfer(
        sources: List<String>,
        targetDir: String,
        mode: String,
        conflict: String,
        filter: FilterSpec? = null,
        baseDir: String? = null,
    ): String = taskId(
        "fs.transfer",
        buildJsonObject {
            put("sources", strings(sources))
            put("targetDir", targetDir)
            put("mode", mode)
            put("conflict", conflict)
            put("filter", filterJson(filter))
            put("baseDir", baseDir)
        },
    )

    /** Not permanent = app trash; places without a trash answer `unsupported`. */
    suspend fun delete(locations: List<String>, permanent: Boolean): String = taskId(
        "fs.delete",
        buildJsonObject {
            put("locations", strings(locations))
            put("permanent", permanent)
        },
    )

    suspend fun properties(locations: List<String>): String =
        taskId("fs.properties", buildJsonObject { put("locations", strings(locations)) })

    /** Local places only; others answer `unsupported` (then use [fetch]). */
    suspend fun open(location: String): OpenTarget =
        Core.request<OpenTarget>("fs.open", buildJsonObject { put("location", location) })

    suspend fun fetch(location: String): String = taskId("fs.fetch", buildJsonObject { put("location", location) })

    suspend fun materialize(locations: List<String>): String =
        taskId("fs.materialize", buildJsonObject { put("locations", strings(locations)) })

    suspend fun edits(): List<EditInfo> = Core.request<List<EditInfo>>("fs.edits")

    /**
     * [mode] `overwrite|copy`. [force] is sent only after the user confirmed "Remote überschreiben"
     * on a reported conflict (contract extension proposed in the K1 report).
     */
    suspend fun uploadEdit(editId: String, mode: String, force: Boolean = false): String = taskId(
        "fs.uploadEdit",
        buildJsonObject {
            put("editId", editId)
            put("mode", mode)
            if (force) put("force", true)
        },
    )

    suspend fun discardEdit(editId: String) {
        Core.call("fs.discardEdit", buildJsonObject { put("editId", editId) })
    }

    /** The core takes ownership of every [ImportFile.fd] and closes it. */
    suspend fun importFiles(files: List<ImportFile>, targetDir: String): String = taskId(
        "fs.import",
        buildJsonObject {
            put("files", Core.json.encodeToJsonElement(ListSerializer(ImportFile.serializer()), files))
            put("targetDir", targetDir)
        },
    )

    /** ZIP; [targetDir] `null` = folder next to the archive. */
    suspend fun extract(location: String, targetDir: String?): String = taskId(
        "fs.extract",
        buildJsonObject {
            put("location", location)
            put("targetDir", targetDir)
        },
    )

    // ---- scan.* / index.* ----

    suspend fun validateFilter(filter: FilterSpec): String? =
        Core.request<FilterCheck>("scan.validate", buildJsonObject { put("filter", filterJson(filter)) }).error

    suspend fun scanStart(location: String, filter: FilterSpec?, showHidden: Boolean): String = taskId(
        "scan.start",
        buildJsonObject {
            put("location", location)
            put("filter", filterJson(filter))
            put("showHidden", showHidden)
        },
    )

    suspend fun scanView(
        taskId: String,
        sort: SortSpec,
        collapsed: Collection<String>,
        offset: Int,
        limit: Int,
        sinceRevision: Long?,
    ): ScanView = Core.request<ScanView>(
        "scan.view",
        buildJsonObject {
            put("taskId", taskId)
            put("sort", sortJson(sort))
            put("collapsed", strings(collapsed))
            put("offset", offset)
            put("limit", limit.coerceIn(1, MAX_WINDOW))
            put("sinceRevision", sinceRevision)
        },
    )

    suspend fun scanIssues(taskId: String): String =
        Core.request<TextAnswer>("scan.issues", buildJsonObject { put("taskId", taskId) }).text

    suspend fun indexStatus(): IndexStatus = Core.request<IndexStatus>("index.status")

    suspend fun indexBuild(): String = taskId("index.build", JsonObject(emptyMap()))

    suspend fun indexSearch(query: String, limit: Int): List<FolderHit> = Core.request<List<FolderHit>>(
        "index.search",
        buildJsonObject {
            put("query", query)
            put("limit", limit)
        },
    )

    // ---- trash.* ----

    suspend fun trashList(): List<TrashItem> = Core.request<List<TrashItem>>("trash.list")

    suspend fun trashRestore(ids: List<String>): RestoreResult =
        Core.request<RestoreResult>("trash.restore", buildJsonObject { put("ids", strings(ids)) })

    suspend fun trashDelete(ids: List<String>): String = taskId("trash.delete", buildJsonObject { put("ids", strings(ids)) })

    suspend fun trashEmpty(): String = taskId("trash.empty", JsonObject(emptyMap()))

    suspend fun trashPurge(olderThanDays: Int): Int =
        Core.request<PurgeResult>("trash.purge", buildJsonObject { put("olderThanDays", olderThanDays) }).removed

    // ---- task.* (api.md §3) ----

    suspend fun taskGet(taskId: String): TaskInfo = Core.request<TaskInfo>("task.get", buildJsonObject { put("id", taskId) })

    suspend fun cancelTask(taskId: String) {
        Core.call("task.cancel", buildJsonObject { put("id", taskId) })
    }

    /** Removes finished tasks (`Core.tasks` reloads itself afterwards). */
    suspend fun clearFinishedTasks() {
        Core.call("task.clear")
    }

    /**
     * Suspends until [taskId] is done, failed or canceled. Relies on task events and re-checks
     * with `task.get` every few seconds so a lost event can never hang the caller.
     */
    suspend fun awaitTask(taskId: String): TaskInfo {
        while (true) {
            val seen = withTimeoutOrNull(TASK_RECHECK_MS) {
                Core.tasks.first { list -> list.any { it.id == taskId && !it.isActive } }
            }?.firstOrNull { it.id == taskId }
            if (seen != null) return seen
            val fresh = taskGet(taskId)
            if (!fresh.isActive) return fresh
        }
    }

    /** Decodes [TaskInfo.result]; `null` when the task has none. */
    fun <T> resultOf(task: TaskInfo, serializer: KSerializer<T>): T? {
        val result = task.result ?: return null
        if (result is JsonNull) return null
        return try {
            Core.json.decodeFromJsonElement(serializer, result)
        } catch (e: IllegalArgumentException) {
            throw CoreException("internal", "Ergebnis von ${task.kind} nicht lesbar: ${e.message}")
        }
    }

    private suspend fun taskId(method: String, args: JsonObject): String = Core.request<TaskRef>(method, args).taskId

    private fun nameArgs(parent: String, name: String): JsonObject = buildJsonObject {
        put("parent", parent)
        put("name", name)
    }

    private fun strings(values: Collection<String>): JsonArray = JsonArray(values.map { JsonPrimitive(it) })

    private fun filterJson(filter: FilterSpec?): JsonElement =
        if (filter == null) JsonNull else Core.json.encodeToJsonElement(FilterSpec.serializer(), filter)

    private fun sortJson(sort: SortSpec): JsonElement = Core.json.encodeToJsonElement(SortSpec.serializer(), sort)
}
