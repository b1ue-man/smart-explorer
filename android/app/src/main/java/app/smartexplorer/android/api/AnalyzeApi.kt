package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.service.TaskKeeper
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
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

    /** "≈ Apps (laut Android)": all installed apps; opens the app list. */
    Apps,

    /** One installed app with Android's figures ([AnalyzeChild.packageName], bytes); opens its breakdown. */
    App,
}

@Serializable
data class AnalyzeChild(
    val name: String,
    val size: Long = 0,
    /** Occupied space ("Belegt"); `null` from an older core = [size]. */
    val allocated: Long? = null,
    val isDir: Boolean = false,
    val childCount: Int = 0,
    /** `dir|file|protected|rest|aggregate|apps|app`; empty or unknown = derived from [isDir]. */
    val kind: String = "",
    /** Package of an app row (`kind: "app"`); its size is [appBytes] + [dataBytes]. */
    @SerialName("package") val packageName: String? = null,
    /** APK, code and the app's `Android/obb` (app rows). */
    val appBytes: Long = 0,
    /** Private data incl. the app's `Android/data` (app rows). */
    val dataBytes: Long = 0,
    /** Cache part of [dataBytes] (app rows). */
    val cacheBytes: Long = 0,
    /** "NTFS-Systemdateien" of the fast NTFS way: occupied by the file system, nothing to open or delete. */
    val systemFiles: Boolean = false,
) {
    val rowKind: AnalyzeKind
        get() = when (kind) {
            "dir" -> AnalyzeKind.Dir
            "file" -> AnalyzeKind.File
            "protected" -> AnalyzeKind.Protected
            "rest" -> AnalyzeKind.Rest
            "aggregate" -> AnalyzeKind.Aggregate
            "apps" -> AnalyzeKind.Apps
            "app" -> AnalyzeKind.App
            else -> if (isDir) AnalyzeKind.Dir else AnalyzeKind.File
        }

    /**
     * A node `analyze.node` can open: a real folder of the result or the app list (estimates,
     * aggregates and single apps cannot).
     */
    val opensFolder: Boolean
        get() = isDir && (rowKind == AnalyzeKind.Dir || rowKind == AnalyzeKind.Protected || rowKind == AnalyzeKind.Apps)

    /** The row reacts to a tap: it opens a folder/the app list, or an app's breakdown. */
    val tappable: Boolean
        get() = !systemFiles && (opensFolder || rowKind == AnalyzeKind.App)

    /** A row from Android's figures that is no entry of the folder (rest, other apps' data, the apps row). */
    val isEstimate: Boolean
        get() = rowKind == AnalyzeKind.Rest || rowKind == AnalyzeKind.Apps || (rowKind == AnalyzeKind.Protected && !isDir)

    /** "Belegt" of the row: [allocated], or [size] where the core sent none. */
    val allocatedBytes: Long
        get() = allocated ?: size
}

/**
 * One analysed folder or the app list; [children] by [sort] descending (`allocated` = "Belegt",
 * `size` = "Größe"), at most 500 (plus display-only rows). [size] includes the estimates below
 * (bytes in both a measured app folder and the apps row counted once), [measured] is what the walk
 * found. [allocated] is the occupied space; [allocatedKnown] false (older peer, backend without the
 * figure) = "Belegt: unbekannt", [allocated] then equals [size].
 */
@Serializable
data class AnalyzeNode(
    val name: String = "",
    val size: Long = 0,
    val allocated: Long? = null,
    val allocatedKnown: Boolean = false,
    /** `allocated|size`: what [children] are sorted and chosen by. */
    val sort: String = "size",
    val measured: Long = 0,
    val isDir: Boolean = true,
    /** `dir|file|apps`; `apps` = the list behind "≈ Apps (laut Android)". */
    val kind: String = "",
    val children: List<AnalyzeChild> = emptyList(),
    /** Place to open in "Dateien", `null` when the core has none (also for the app list). */
    val location: String? = null,
    /** Volume figures of the storing host (root node only; null = unknown). */
    val volumeTotal: Long? = null,
    val volumeFree: Long? = null,
    val remote: Boolean = false,
) {
    val isAppList: Boolean
        get() = kind == "apps"
}

/** One file of "Größte Dateien" ([path] below the shown folder, [location] = the folder holding it). */
@Serializable
data class AnalyzeLargestFile(val path: String = "", val size: Long = 0, val allocated: Long = 0, val location: String? = null)

/** One extension of "Nach Dateityp". */
@Serializable
data class AnalyzeTypeTotal(val extension: String = "", val files: Long = 0, val size: Long = 0, val allocated: Long = 0)

/**
 * `analyze.overview`: "Größte Dateien" and "Nach Dateityp" of a folder with its subfolders, by
 * [sort]. Files the core folded into "… N weitere Eintraege" count only in [aggregatedFiles] etc.;
 * [aggregatedNote] says so (`null` = none).
 */
@Serializable
data class AnalyzeOverview(
    val sort: String = "size",
    val allocatedKnown: Boolean = false,
    val largest: List<AnalyzeLargestFile> = emptyList(),
    val types: List<AnalyzeTypeTotal> = emptyList(),
    val aggregatedFiles: Long = 0,
    val aggregatedSize: Long = 0,
    val aggregatedAllocated: Long = 0,
    val aggregatedNote: String? = null,
)

/**
 * Read problems of an analysis; [count] excludes the areas Android locks for every app, which are
 * counted in [protectedCount] and described in [protectedText]. [notes] are remarks of the walk or
 * of the device that analysed a remote place (summarised detail, an older analysis path, areas it
 * protects); they never make the result incomplete.
 */
@Serializable
data class AnalyzeIssues(
    val count: Int = 0,
    val text: String = "",
    val notes: List<String> = emptyList(),
    val protectedCount: Long = 0,
    val protectedText: String = "",
)

/**
 * Platform figures of the volume that holds the analysed root (`analyze.start.platform`):
 * [volumeUsedBytes] from `StatFs`, [otherAppsBytes] = `ExternalStorageStats.getAppBytes()` of the
 * primary volume (only with usage access) and [apps], every installed app with bytes on the whole
 * primary volume (only with usage access). `null` = unknown.
 */
data class AnalyzePlatform(val volumeUsedBytes: Long?, val otherAppsBytes: Long?, val apps: List<AnalyzeApp>? = null)

/** One installed app's storage (`StorageStatsManager.queryStatsForPackage`, `analyze.start.platform.apps`). */
data class AnalyzeApp(
    val packageName: String,
    val label: String,
    val appBytes: Long,
    val dataBytes: Long,
    val cacheBytes: Long,
)

@Serializable
data class DuplicateItem(val location: String, val mtimeMs: Long = 0)

/**
 * Files with equal content; [size] is the size of one copy, [reclaimable] the occupied space that
 * removing all but one copy frees (`null` from an older core: [size] per further copy).
 */
@Serializable
data class DuplicateGroup(
    val size: Long = 0,
    val reclaimable: Long? = null,
    val items: List<DuplicateItem> = emptyList(),
    /** The core retained a whole-content SHA-256 for a safe remote trash request. */
    val contentVerified: Boolean = false,
) {
    /** What removing the further copies frees ("Belegt"). */
    val freedBytes: Long
        get() = reclaimable ?: (size * (items.size - 1).coerceAtLeast(0))
}

/**
 * Totals of a finished duplicate search: [files]/[bytes] walked, [candidates] files at or above
 * the minimum size, [compared] of them shared their size with another one and were compared,
 * [groups] found, [protectedCount] areas Android locks, [errorCount] unreadable paths ([errorText]
 * lists them) and [limit], why the search saw or compared less than everything (one reason per
 * line, worded by the core; `null` = complete).
 */
@Serializable
data class ReclaimSummary(
    val files: Long = 0,
    val bytes: Long = 0,
    val candidates: Long = 0,
    val compared: Long = 0,
    val groups: Long = 0,
    val protectedCount: Long = 0,
    val protectedText: String = "",
    val errorCount: Long = 0,
    val errorText: String = "",
    val limit: String? = null,
    val remote: Boolean = false,
    val canRecycle: Boolean = false,
    val recycleNote: String = "",
    /** The search measured the copies' occupied bytes; `null` from an older core (2026-11-18). */
    val allocatedKnown: Boolean? = null,
) {
    /** "Belegt" of the result is known: the core says so; an older core only for a local search. */
    val occupiedKnown: Boolean
        get() = allocatedKnown ?: !remote
}

@Serializable
data class RecycleResult(val moved: List<String> = emptyList())

/** Suspending wrappers for api.md §4.9; every call throws [CoreException] on errors. */
object AnalyzeApi {
    /**
     * Starts the analysis; progress: `doneItems` files, `doneBytes`, `message` = phase and current
     * folder, one fact per line. A remote place is analysed by the device that holds it: the
     * message names its phases, and while the finished result is transferred `doneBytes` of
     * `totalBytes` is that transfer. [platform] only for a local root on a mounted volume; unknown
     * figures are left out.
     */
    suspend fun start(location: String, platform: AnalyzePlatform? = null): String = started(
        Core.request<Started>(
            "analyze.start",
            buildJsonObject {
                put("location", location)
                if (platform != null) put("platform", platformJson(platform))
            },
        ),
    )

    /** `analyze.start.platform` (api.md §4.9); unknown figures are left out. */
    fun platformJson(platform: AnalyzePlatform): JsonObject = buildJsonObject {
        platform.volumeUsedBytes?.let { put("volumeUsedBytes", it) }
        platform.otherAppsBytes?.let { put("otherAppsBytes", it) }
        platform.apps?.let { apps ->
            put(
                "apps",
                buildJsonArray {
                    apps.forEach { app ->
                        add(
                            buildJsonObject {
                                put("package", app.packageName)
                                put("label", app.label)
                                put("appBytes", app.appBytes)
                                put("dataBytes", app.dataBytes)
                                put("cacheBytes", app.cacheBytes)
                            },
                        )
                    }
                },
            )
        }
    }

    /**
     * Folder at [path] (names below the analysed root; empty = the root), its children sorted and
     * chosen by [sort] (`allocated` | `size`; the core uses `size` where "Belegt" is unknown).
     */
    suspend fun node(taskId: String, path: List<String>, sort: String = "allocated"): AnalyzeNode =
        Core.request<AnalyzeNode>(
            "analyze.node",
            buildJsonObject {
                put("taskId", taskId)
                put("path", JsonArray(path.map { JsonPrimitive(it) }))
                put("sort", sort)
            },
        )

    /** "Größte Dateien" and "Nach Dateityp" of the folder at [path] (with its subfolders). */
    suspend fun overview(taskId: String, path: List<String>, sort: String = "allocated"): AnalyzeOverview =
        Core.request<AnalyzeOverview>(
            "analyze.overview",
            buildJsonObject {
                put("taskId", taskId)
                put("path", JsonArray(path.map { JsonPrimitive(it) }))
                put("sort", sort)
            },
        )

    suspend fun issues(taskId: String): AnalyzeIssues =
        Core.request<AnalyzeIssues>("analyze.issues", buildJsonObject { put("taskId", taskId) })

    /**
     * Frees the kept result of [taskId] (analysis or duplicate search) once the page shows it no
     * more; `false` when the core had nothing kept for it.
     */
    suspend fun release(taskId: String): Boolean =
        Core.request<Released>("analyze.release", buildJsonObject { put("taskId", taskId) }).released

    /** Duplicate search for files of at least [minSize] bytes; progress `message` = phase. */
    suspend fun reclaimStart(location: String, minSize: Long): String = started(
        Core.request<Started>(
            "reclaim.start",
            buildJsonObject {
                put("location", location)
                put("minSize", minSize)
            },
        ),
    )

    /** All groups of the finished search. */
    suspend fun reclaimGroups(taskId: String): List<DuplicateGroup> =
        Core.request<List<DuplicateGroup>>("reclaim.groups", buildJsonObject { put("taskId", taskId) })

    suspend fun reclaimSummary(taskId: String): ReclaimSummary =
        Core.request<ReclaimSummary>("reclaim.summary", buildJsonObject { put("taskId", taskId) })

    /** The source search, not caller-provided hashes, authorises each selected copy. */
    suspend fun reclaimRecycle(taskId: String, locations: List<String>): String = started(
        Core.request<Started>("reclaim.recycle", buildJsonObject {
            put("taskId", taskId)
            put("locations", JsonArray(locations.map { JsonPrimitive(it) }))
        }),
    )

    suspend fun recycleResult(taskId: String): RecycleResult =
        Core.request<RecycleResult>("reclaim.recycleResult", buildJsonObject { put("taskId", taskId) })

    /**
     * A task against another device keeps the CPU awake for as long as it runs: with the screen off
     * the phone would otherwise suspend, the other side's connection would time out and the
     * result would be partial or lost.
     */
    private fun started(answer: Started): String {
        if (answer.remote) TaskKeeper.keepCpuAwake(answer.taskId)
        return answer.taskId
    }

    /** Answer of `analyze.start` / `reclaim.start`; [remote] = the task works against another device. */
    @Serializable
    private data class Started(val taskId: String, val remote: Boolean = false)

    @Serializable
    private data class Released(val released: Boolean = false)
}
