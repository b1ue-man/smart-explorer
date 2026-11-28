package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// "In Dateien ersetzen", api.md §4.14 (search.content.replace.*). Ranges are UTF-16 units of the
// line text the search delivered, as in search.content.poll.

/** A hit line of an earlier search that a replacement takes, exactly as the poll delivered it. */
@Serializable
data class ReplaceHit(
    val rel: String,
    val line: Long,
    val text: String = "",
    val ranges: List<List<Int>> = emptyList(),
    /** A cut line: the core replaces every match of that line. */
    val truncated: Boolean = false,
    val size: Long = 0,
    val mtimeMs: Long = 0,
)

/** `file` (a file with [replacements] places) or `skipped` (left unchanged, [reason] in German). */
@Serializable
data class ReplaceEventDto(
    val type: String,
    val rel: String = "",
    val location: String = "",
    val replacements: Long = 0,
    val reason: String? = null,
)

@Serializable
data class ReplaceSummaryDto(
    val filesSearched: Long = 0,
    val filesChanged: Long = 0,
    val replacements: Long = 0,
    val skipped: Long = 0,
    /** The journal entry `search.content.replace.undo` restores; null when nothing was backed up. */
    val undoId: String? = null,
)

@Serializable
data class ReplacePoll(
    val events: List<ReplaceEventDto> = emptyList(),
    val next: Long = 0,
    val files: Long = 0,
    val replacements: Long = 0,
    val done: Boolean = false,
    val canceled: Boolean = false,
    val error: String? = null,
    val summary: ReplaceSummaryDto? = null,
)

/** One run of the replacement journal (newest first); [state] `running|applied|partly_undone|undone`. */
@Serializable
data class ReplaceHistoryEntry(
    val undoId: String,
    val createdMs: Long = 0,
    val location: String = "",
    val root: String = "",
    val pattern: String = "",
    val replacement: String = "",
    val mode: String = "normal",
    val filesChanged: Long = 0,
    val replacements: Long = 0,
    val state: String = "",
)

/** A file "Rückgängig" left as it is (changed since the run), with the German reason. */
@Serializable
data class UndoSkipped(val rel: String, val reason: String = "")

/** What `search.content.replace.undo` did. */
@Serializable
data class UndoResult(val restored: List<String> = emptyList(), val skipped: List<UndoSkipped> = emptyList())

@Serializable
internal data class ReplaceHistory(val entries: List<ReplaceHistoryEntry> = emptyList())

object ReplaceApi {
    /**
     * Starts a replacement below [location]: every match ([hits] null) or the given hit lines of an
     * earlier search. [dryRun] only counts. Returns the task id.
     */
    suspend fun start(
        location: String,
        query: ContentSearchQuery,
        replacement: String,
        hits: List<ReplaceHit>?,
        dryRun: Boolean,
    ): String {
        val fields = Core.json.encodeToJsonElement(ContentSearchQuery.serializer(), query) as JsonObject
        val extra = buildMap<String, JsonElement> {
            put("location", JsonPrimitive(location))
            put("replacement", JsonPrimitive(replacement))
            put("selection", JsonPrimitive(if (hits == null) "all" else "hits"))
            put("dryRun", JsonPrimitive(dryRun))
            if (hits != null) {
                put("hits", JsonArray(hits.map { Core.json.encodeToJsonElement(ReplaceHit.serializer(), it) }))
            }
        }
        return Core.request<TaskRef>("search.content.replace.start", JsonObject(fields + extra)).taskId
    }

    suspend fun poll(taskId: String, since: Long): ReplacePoll = Core.request<ReplacePoll>(
        "search.content.replace.poll",
        buildJsonObject {
            put("taskId", taskId)
            put("since", since)
        },
    )

    suspend fun cancel(taskId: String) {
        Core.call("search.content.replace.cancel", buildJsonObject { put("taskId", taskId) })
    }

    /** Restores the backups of the run [undoId]; returns when every file is handled. */
    suspend fun undo(undoId: String): UndoResult =
        Core.request<UndoResult>("search.content.replace.undo", buildJsonObject { put("undoId", undoId) })

    suspend fun history(): List<ReplaceHistoryEntry> =
        Core.request<ReplaceHistory>("search.content.replace.history").entries
}
