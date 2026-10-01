package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// Answers of api.md §4.9 (storage analysis, duplicates); field names exactly as the contract.

/** Row kind of an [AnalyzeChild] (`kind`, api.md §4.9). */
enum class AnalyzeKind {
    Dir,
    File,

    /** Folder Android locks for every app (`<Volume>/Android/data|obb` and below). */
    Protected,

    /** Display-only estimate from platform figures (e.g. "≈ Nicht einzeln erfasst"); not a real entry. */
    Rest,

    /** Several entries the core shows as one row. */
    Aggregate,
}

@Serializable
data class AnalyzeChild(
    val name: String,
    val size: Long = 0,
    val isDir: Boolean = false,
    val childCount: Int = 0,
    /** `dir|file|protected|rest|aggregate`; empty or unknown = derived from [isDir]. */
    val kind: String = "",
) {
    val rowKind: AnalyzeKind
        get() = when (kind) {
            "dir" -> AnalyzeKind.Dir
            "file" -> AnalyzeKind.File
            "protected" -> AnalyzeKind.Protected
            "rest" -> AnalyzeKind.Rest
            "aggregate" -> AnalyzeKind.Aggregate
            else -> if (isDir) AnalyzeKind.Dir else AnalyzeKind.File
        }

    /** A real folder of the result that `analyze.node` can open (estimates and aggregates cannot). */
    val opensFolder: Boolean
        get() = isDir && (rowKind == AnalyzeKind.Dir || rowKind == AnalyzeKind.Protected)
}

/** One analysed folder; [children] by size descending, at most 500 (plus display-only rows). */
@Serializable
data class AnalyzeNode(
    val name: String = "",
    val size: Long = 0,
    val isDir: Boolean = true,
    val children: List<AnalyzeChild> = emptyList(),
    /** Place to open in "Dateien", `null` when the core has none. */
    val location: String? = null,
)

/**
 * Read problems of an analysis; [count] excludes the areas Android locks for every app, which are
 * counted in [protectedCount] and described in [protectedText].
 */
@Serializable
data class AnalyzeIssues(
    val count: Int = 0,
    val text: String = "",
    val protectedCount: Long = 0,
    val protectedText: String = "",
)

/**
 * Platform figures of the volume that holds the analysed root (`analyze.start.platform`):
 * [volumeUsedBytes] from `StatFs`, [otherAppsBytes] = `ExternalStorageStats.getAppBytes()` of the
 * primary volume (only with usage access). `null` = unknown.
 */
data class AnalyzePlatform(val volumeUsedBytes: Long?, val otherAppsBytes: Long?)

@Serializable
data class DuplicateItem(val location: String, val mtimeMs: Long = 0)

/** Files with equal content; [size] is the size of one copy. */
@Serializable
data class DuplicateGroup(val size: Long = 0, val items: List<DuplicateItem> = emptyList())

/**
 * Totals of a finished duplicate search: [files]/[bytes] walked, [candidates] files at or above
 * the minimum size, [groups] found, [protectedCount] areas Android locks, [errorCount] unreadable
 * paths ([errorText] lists them) and [limit], the reason the walk stopped early (`null` = complete).
 */
@Serializable
data class ReclaimSummary(
    val files: Long = 0,
    val bytes: Long = 0,
    val candidates: Long = 0,
    val groups: Long = 0,
    val protectedCount: Long = 0,
    val errorCount: Long = 0,
    val errorText: String = "",
    val limit: String? = null,
)

/** Suspending wrappers for api.md §4.9; every call throws [CoreException] on errors. */
object AnalyzeApi {
    /**
     * Starts the analysis; progress: `doneItems` files, `doneBytes`, `message` = phase and current
     * folder. [platform] only for a local root on a mounted volume; unknown figures are left out.
     */
    suspend fun start(location: String, platform: AnalyzePlatform? = null): String = Core.request<TaskId>(
        "analyze.start",
        buildJsonObject {
            put("location", location)
            if (platform != null) {
                put(
                    "platform",
                    buildJsonObject {
                        platform.volumeUsedBytes?.let { put("volumeUsedBytes", it) }
                        platform.otherAppsBytes?.let { put("otherAppsBytes", it) }
                    },
                )
            }
        },
    ).taskId

    /** Folder at [path] (names below the analysed root; empty = the root). */
    suspend fun node(taskId: String, path: List<String>): AnalyzeNode = Core.request<AnalyzeNode>(
        "analyze.node",
        buildJsonObject {
            put("taskId", taskId)
            put("path", JsonArray(path.map { JsonPrimitive(it) }))
        },
    )

    suspend fun issues(taskId: String): AnalyzeIssues =
        Core.request<AnalyzeIssues>("analyze.issues", buildJsonObject { put("taskId", taskId) })

    /** Duplicate search for files of at least [minSize] bytes; progress `message` = phase. */
    suspend fun reclaimStart(location: String, minSize: Long): String = Core.request<TaskId>(
        "reclaim.start",
        buildJsonObject {
            put("location", location)
            put("minSize", minSize)
        },
    ).taskId

    /** All groups of the finished search. */
    suspend fun reclaimGroups(taskId: String): List<DuplicateGroup> =
        Core.request<List<DuplicateGroup>>("reclaim.groups", buildJsonObject { put("taskId", taskId) })

    suspend fun reclaimSummary(taskId: String): ReclaimSummary =
        Core.request<ReclaimSummary>("reclaim.summary", buildJsonObject { put("taskId", taskId) })

    @Serializable
    private data class TaskId(val taskId: String)
}
