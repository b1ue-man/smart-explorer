package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// Answers of api.md §4.9 (storage analysis, duplicates); field names exactly as the contract.

@Serializable
data class AnalyzeChild(
    val name: String,
    val size: Long = 0,
    val isDir: Boolean = false,
    val childCount: Int = 0,
)

/** One analysed folder; [children] by size descending, at most 500. */
@Serializable
data class AnalyzeNode(
    val name: String = "",
    val size: Long = 0,
    val isDir: Boolean = true,
    val children: List<AnalyzeChild> = emptyList(),
    /** Place to open in "Dateien", `null` when the core has none. */
    val location: String? = null,
)

@Serializable
data class AnalyzeIssues(val count: Int = 0, val text: String = "")

@Serializable
data class DuplicateItem(val location: String, val mtimeMs: Long = 0)

/** Files with equal content; [size] is the size of one copy. */
@Serializable
data class DuplicateGroup(val size: Long = 0, val items: List<DuplicateItem> = emptyList())

/** Suspending wrappers for api.md §4.9; every call throws [CoreException] on errors. */
object AnalyzeApi {
    /** Starts the analysis; progress: `doneItems` files, `doneBytes`. */
    suspend fun start(location: String): String =
        Core.request<TaskId>("analyze.start", buildJsonObject { put("location", location) }).taskId

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

    /** Duplicate search for files of at least [minSize] bytes. */
    suspend fun reclaimStart(location: String, minSize: Long): String = Core.request<TaskId>(
        "reclaim.start",
        buildJsonObject {
            put("location", location)
            put("minSize", minSize)
        },
    ).taskId

    suspend fun reclaimGroups(taskId: String): List<DuplicateGroup> =
        Core.request<List<DuplicateGroup>>("reclaim.groups", buildJsonObject { put("taskId", taskId) })

    @Serializable
    private data class TaskId(val taskId: String)
}
