package app.smartexplorer.android.core

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

// Shared JSON types, field names exactly as api.md §2 (camelCase). Defaults keep decoding
// tolerant; `Core.json` encodes defaults, so requests always carry every field.

/** One directory entry; `depth > 0` only in recursive scan views. */
@Serializable
data class Entry(
    val name: String,
    val location: String,
    val isDir: Boolean,
    val isLink: Boolean = false,
    val size: Long = 0,
    val mtimeMs: Long = 0,
    val hidden: Boolean = false,
    /** Warning text for problematic names, `null` when the name is fine. */
    val problem: String? = null,
    /** `dir|image|video|audio|text|archive|document|apk|other` */
    val kind: String = "other",
    val ext: String = "",
    val depth: Int = 0,
    val hasChildren: Boolean = false,
    val expanded: Boolean = false,
)

@Serializable
data class Crumb(
    val label: String,
    val location: String,
)

@Serializable
data class Root(
    val id: String,
    val label: String,
    val subtitle: String? = null,
    val location: String,
    /** `storage|favorite|recent|connection|gdrive|device|room|trash` */
    val kind: String,
    val removable: Boolean = false,
)

/** api.md `Filter`. */
@Serializable
data class FilterSpec(
    val text: String = "",
    /** `substring|glob|regex` */
    val mode: String = "substring",
    val extensions: String = "",
    val sizeMin: Long? = null,
    val sizeMax: Long? = null,
    val mtimeMinMs: Long? = null,
    val mtimeMaxMs: Long? = null,
    val files: Boolean = true,
    val dirs: Boolean = true,
    val hidden: Boolean = false,
    val problemOnly: Boolean = false,
)

/** api.md `Sort`. */
@Serializable
data class SortSpec(
    /** `name|size|mtime|type` */
    val key: String = "name",
    val desc: Boolean = false,
    val dirsFirst: Boolean = true,
)

/** api.md `Task`. */
@Serializable
data class TaskInfo(
    val id: String,
    val kind: String,
    val title: String,
    /** `queued|running|done|failed|canceled` */
    val state: String,
    val doneBytes: Long = 0,
    val totalBytes: Long = 0,
    val doneItems: Long = 0,
    val totalItems: Long = 0,
    val rateBps: Long = 0,
    val message: String? = null,
    val errors: List<TaskError> = emptyList(),
    val result: JsonElement? = null,
    val startedMs: Long = 0,
    val finishedMs: Long? = null,
) {
    /** `true` while the task is queued or running. */
    val isActive: Boolean
        get() = state == "queued" || state == "running"
}

@Serializable
data class TaskError(
    val path: String,
    val message: String,
)

/** One mounted storage volume as sent in the init config and `sys.volumes` (api.md §1, §4.1). */
@Serializable
data class VolumeInfo(
    val path: String,
    val label: String,
    val primary: Boolean,
    val removable: Boolean,
)
