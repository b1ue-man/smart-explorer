package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// Answers of api.md §4.14 (search.content.*), §4.15 (text.*) and §4.16 (languages.*). Every
// column and range is in UTF-16 units (Kotlin String indices), every color 32-bit ARGB.

/** Fields of "In Dateien suchen" (Notepad++ "Find in Files" plus the two folder-path regexes). */
@Serializable
data class ContentSearchQuery(
    val pattern: String,
    /** `normal|extended|regex` */
    val mode: String = "normal",
    val matchCase: Boolean = false,
    val wholeWord: Boolean = false,
    val dotMatchesNewline: Boolean = false,
    val filters: String = "",
    val folderInclude: String? = null,
    val folderExclude: String? = null,
    val subfolders: Boolean = true,
    val hidden: Boolean = false,
    val contextLines: Int = 0,
    val maxHits: Long = 0,
    val filesOnly: Boolean = false,
    /** Skip what `.gitignore` (in a Git work tree), `.ignore` and `.git/info/exclude` exclude. */
    val respectIgnoreFiles: Boolean = false,
)

/** One line of a hit event; [ranges] are `[start, end]` pairs in [text]. */
@Serializable
data class ContentLine(
    val line: Long,
    val text: String = "",
    val ranges: List<List<Int>> = emptyList(),
    val truncated: Boolean = false,
    val context: Boolean = false,
)

/** `hits` or `skipped` (api.md §4.14). */
@Serializable
data class ContentEvent(
    val type: String,
    val rel: String = "",
    val location: String = "",
    val size: Long = 0,
    val mtimeMs: Long = 0,
    val last: Boolean = false,
    val lines: List<ContentLine> = emptyList(),
    /** `binary|unreadable|unlistable` for `skipped` */
    val reason: String? = null,
    val detail: String? = null,
)

@Serializable
data class ContentSummary(
    val filesSearched: Long = 0,
    val filesMatched: Long = 0,
    val hits: Long = 0,
    val bytes: Long = 0,
    val skipped: Long = 0,
    val limited: Boolean = false,
)

@Serializable
data class ContentPoll(
    val events: List<ContentEvent> = emptyList(),
    val next: Long = 0,
    val files: Long = 0,
    val bytes: Long = 0,
    val hits: Long = 0,
    val done: Boolean = false,
    val canceled: Boolean = false,
    val error: String? = null,
    val summary: ContentSummary? = null,
)

@Serializable
data class StyledSpanDto(
    val start: Int,
    val end: Int,
    val fg: Long,
    val bg: Long? = null,
    val bold: Boolean = false,
    val italic: Boolean = false,
    val underline: Boolean = false,
)

@Serializable
data class StyledLineDto(
    /** Zero-based line number (`text.lines`); `null` in Markdown code blocks. */
    val number: Long? = null,
    val text: String = "",
    val spans: List<StyledSpanDto> = emptyList(),
)

@Serializable
data class ThemeColorsDto(
    val theme: String = "",
    val background: Long = 0xFFFFFFFF,
    val foreground: Long = 0xFF000000,
    val selection: Long = 0x40000000,
    val lineHighlight: Long = 0xFFFFFFFF,
    val gutter: Long = 0xFF000000,
)

@Serializable
data class TextOpened(
    val docId: String,
    val name: String = "",
    val location: String = "",
    val prefix: String = "",
    val path: String = "",
    val size: Long = 0,
    val encoding: String = "",
    val encodingKey: String = "",
    val language: String = "",
    val markdown: Boolean = false,
    val lines: Long = 0,
    val complete: Boolean = false,
)

@Serializable
data class LineEndings(val crlf: Long = 0, val lf: Long = 0, val cr: Long = 0)

@Serializable
data class TextStatus(
    val lines: Long = 0,
    val complete: Boolean = false,
    val bytesDone: Long = 0,
    val totalBytes: Long = 0,
    val encoding: String = "",
    val encodingKey: String = "",
    val endings: LineEndings = LineEndings(),
    val lineEnding: String? = null,
    val mixedEndings: Boolean = false,
    val error: String? = null,
)

@Serializable
data class TextLines(
    val first: Long = 0,
    val language: String = "",
    val lines: List<StyledLineDto> = emptyList(),
    val known: Long = 0,
    val complete: Boolean = false,
    val colors: ThemeColorsDto = ThemeColorsDto(),
)

@Serializable
data class TextFound(
    val found: Boolean = false,
    val line: Long = 0,
    val start: Int = 0,
    val end: Int = 0,
    val wrapped: Boolean = false,
)

@Serializable
data class LanguageInfoDto(val name: String, val extensions: List<String> = emptyList())

@Serializable
data class LanguagePackDto(
    val name: String,
    /** `builtin|added` */
    val origin: String = "builtin",
    val languages: List<LanguageInfoDto> = emptyList(),
    val license: String = "",
    val path: String? = null,
    val shadowedBy: String? = null,
    val problems: List<String> = emptyList(),
)

/** Kind of an inline run: `text|code|line_break|image|footnote_ref|html`. */
@Serializable
data class MdRunKindDto(val type: String = "text", val url: String = "", val title: String = "")

@Serializable
data class MdRunDto(
    val text: String = "",
    val kind: MdRunKindDto = MdRunKindDto(),
    val strong: Boolean = false,
    val emphasis: Boolean = false,
    val strikethrough: Boolean = false,
    val link: String? = null,
)

@Serializable
data class MdListItemDto(val task: Boolean? = null, val blocks: List<MdBlockDto> = emptyList())

/**
 * One block of the core's Markdown model (`type`: heading, paragraph, list, quote, code, table,
 * rule, image, html, footnote_definition); one flat class, so an unknown future type decodes too.
 */
@Serializable
data class MdBlockDto(
    val type: String,
    val level: Int = 0,
    val runs: List<MdRunDto> = emptyList(),
    val start: Long? = null,
    val items: List<MdListItemDto> = emptyList(),
    /** Quote: `note|tip|important|warning|caution` (GitHub alerts) or `null`. */
    val kind: String? = null,
    val blocks: List<MdBlockDto> = emptyList(),
    val language: String? = null,
    val highlightedAs: String = "",
    val lines: List<StyledLineDto> = emptyList(),
    /** Table: `none|left|center|right` per column. */
    val alignments: List<String> = emptyList(),
    val header: List<List<MdRunDto>> = emptyList(),
    val rows: List<List<List<MdRunDto>>> = emptyList(),
    val url: String = "",
    val title: String = "",
    val alt: String = "",
    val text: String = "",
    val label: String = "",
)

@Serializable
data class MdDocumentDto(val blocks: List<MdBlockDto> = emptyList())

@Serializable
internal data class Closed(val closed: Boolean = false)

object TextApi {
    // ---- search.content.* ----

    suspend fun searchStart(location: String, query: ContentSearchQuery): String {
        val fields = Core.json.encodeToJsonElement(ContentSearchQuery.serializer(), query) as JsonObject
        val args = JsonObject(fields + ("location" to JsonPrimitive(location)))
        return Core.request<TaskRef>("search.content.start", args).taskId
    }

    suspend fun searchPoll(taskId: String, since: Long): ContentPoll = Core.request<ContentPoll>(
        "search.content.poll",
        buildJsonObject {
            put("taskId", taskId)
            put("since", since)
        },
    )

    suspend fun searchCancel(taskId: String) {
        Core.call("search.content.cancel", buildJsonObject { put("taskId", taskId) })
    }

    // ---- text.* ----

    suspend fun open(location: String): TextOpened =
        Core.request<TextOpened>("text.open", buildJsonObject { put("location", location) })

    suspend fun status(docId: String): TextStatus =
        Core.request<TextStatus>("text.status", buildJsonObject { put("docId", docId) })

    suspend fun lines(docId: String, first: Long, count: Int, dark: Boolean, language: String?): TextLines =
        Core.request<TextLines>(
            "text.lines",
            buildJsonObject {
                put("docId", docId)
                put("first", first)
                put("count", count)
                put("theme", if (dark) "dark" else "light")
                if (language != null) put("language", language)
            },
        )

    suspend fun find(
        docId: String,
        pattern: String,
        regex: Boolean,
        matchCase: Boolean,
        fromLine: Long,
        fromColumn: Int,
        backward: Boolean,
    ): TextFound = Core.request<TextFound>(
        "text.find",
        buildJsonObject {
            put("docId", docId)
            put("pattern", pattern)
            put("mode", if (regex) "regex" else "normal")
            put("matchCase", matchCase)
            put("fromLine", fromLine)
            put("fromColumn", fromColumn)
            put("backward", backward)
            put("wrap", true)
        },
    )

    suspend fun markdown(docId: String, dark: Boolean): MdDocumentDto = Core.request<MdDocumentDto>(
        "text.markdown",
        buildJsonObject {
            put("docId", docId)
            put("theme", if (dark) "dark" else "light")
        },
    )

    suspend fun close(docId: String): Boolean =
        Core.request<Closed>("text.close", buildJsonObject { put("docId", docId) }).closed

    suspend fun languages(): List<LanguageInfoDto> = Core.request<List<LanguageInfoDto>>("text.languages")

    // ---- languages.* ----

    suspend fun packs(): List<LanguagePackDto> = Core.request<List<LanguagePackDto>>("languages.list")

    suspend fun addPack(fileName: String, bytesBase64: String): LanguagePackDto = Core.request<LanguagePackDto>(
        "languages.add",
        buildJsonObject {
            put("fileName", fileName)
            put("bytesBase64", bytesBase64)
        },
    )

    suspend fun removePack(name: String) {
        Core.call("languages.remove", buildJsonObject { put("name", name) })
    }
}
