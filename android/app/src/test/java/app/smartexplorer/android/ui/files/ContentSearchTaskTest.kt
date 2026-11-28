package app.smartexplorer.android.ui.files

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.SpanStyle
import app.smartexplorer.android.api.ContentEvent
import app.smartexplorer.android.api.ContentLine
import app.smartexplorer.android.api.ContentPoll
import app.smartexplorer.android.api.ContentSearchQuery
import app.smartexplorer.android.api.ContentSummary
import app.smartexplorer.android.core.Core
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Result list of "In Dateien suchen" (api.md §4.14) built from `search.content.poll` answers. */
class ContentSearchTaskTest {
    private fun hit(line: Long, text: String, vararg ranges: Pair<Int, Int>, context: Boolean = false) =
        ContentLine(line = line, text = text, ranges = ranges.map { listOf(it.first, it.second) }, context = context)

    private fun hits(rel: String, vararg lines: ContentLine, last: Boolean = true) =
        ContentEvent(type = "hits", rel = rel, location = "/storage/emulated/0/Projekt/$rel", lines = lines.toList(), last = last)

    @Test
    fun content_search_task_header_counts_hits_files_and_searched() {
        val results = ContentResults(ContentSearchQuery(pattern = "foo"))
        results.apply(
            ContentPoll(
                events = listOf(
                    hits("a.txt", hit(1, "foo", 0 to 3), hit(2, "davor", context = true), hit(3, "foo foo", 0 to 3, 4 to 7)),
                    hits("sub/b.rs", hit(9, "let foo = 1;", 4 to 7)),
                ),
                next = 2,
                files = 10,
            ),
        )
        assertEquals("Suche „foo“ (3 Treffer in 2 Dateien von 10 durchsuchten) [Normal]", results.header())
        // The core's totals of the end win over the running count.
        results.apply(ContentPoll(done = true, summary = ContentSummary(filesSearched = 12, filesMatched = 2, hits = 3)))
        assertEquals("Suche „foo“ (3 Treffer in 2 Dateien von 12 durchsuchten) [Normal]", results.header())
        assertTrue(results.done)

        val one = ContentResults(ContentSearchQuery(pattern = "x", matchCase = true, wholeWord = true))
        one.apply(ContentPoll(events = listOf(hits("a.txt", hit(1, "x", 0 to 1))), files = 1))
        assertEquals("Suche „x“ (1 Treffer in 1 Datei von 1 durchsuchten) [Normal: Groß/Klein/Wort]", one.header())
        // Notepad++ disables "whole word" for regular expressions.
        assertEquals("[Regex: . = Zeilenumbruch]", modeLabel(ContentSearchQuery(pattern = "a.b", mode = "regex", wholeWord = true, dotMatchesNewline = true)))
        assertEquals("[Erweitert]", modeLabel(ContentSearchQuery(pattern = "\\t", mode = "extended")))
    }

    @Test
    fun content_search_task_events_group_by_file_in_arrival_order() {
        val results = ContentResults(ContentSearchQuery(pattern = "foo"))
        results.apply(ContentPoll(events = listOf(hits("a.txt", hit(1, "foo", 0 to 3), last = false), hits("b.txt", hit(4, "foo", 0 to 3)))))
        results.apply(ContentPoll(events = listOf(hits("a.txt", hit(80, "foo", 0 to 3)))))
        val files = results.files
        assertEquals(listOf("a.txt", "b.txt"), files.map { it.rel })
        assertEquals(listOf(1L, 80L), files[0].lines.map { it.line })
        assertEquals(2, files[0].hits)
        assertEquals("/storage/emulated/0/Projekt/a.txt", files[0].location)
        assertEquals(1L, files[0].firstHit?.line)
        assertEquals(3, results.hits)
    }

    @Test
    fun content_search_task_collapsed_files_show_only_their_head() {
        val results = ContentResults(ContentSearchQuery(pattern = "foo"))
        results.apply(
            ContentPoll(
                events = listOf(
                    hits("a.txt", hit(1, "foo", 0 to 3), hit(2, "foo", 0 to 3)),
                    hits("b.txt", hit(4, "foo", 0 to 3)),
                    ContentEvent(type = "skipped", rel = "bild.dat", reason = "binary"),
                ),
            ),
        )
        val open = results.rows(emptySet())
        assertEquals(
            listOf("f:a.txt", "l:a.txt:0", "l:a.txt:1", "f:b.txt", "l:b.txt:0", "s", "s:0"),
            open.map { it.key },
        )
        val folded = results.rows(setOf("a.txt", SKIPPED_KEY))
        assertEquals(listOf("f:a.txt", "f:b.txt", "l:b.txt:0", "s"), folded.map { it.key })
        assertEquals(false, (folded[0] as ResultRow.File).expanded)
        assertEquals(1, (folded[3] as ResultRow.SkippedHeader).count)
        // Keys are unique, as the LazyColumn needs them.
        assertEquals(open.size, open.map { it.key }.toSet().size)
    }

    @Test
    fun content_search_task_hit_text_marks_ranges_after_the_prefix() {
        val mark = SpanStyle(background = Color.Yellow)
        // "Größe " is six UTF-16 units: the core sends UTF-16 ranges.
        val text = hitText(hit(2, "Größe hallo", 6 to 11), mark)
        assertEquals("Zeile 2: Größe hallo", text.text)
        val styles = text.spanStyles
        assertEquals(1, styles.size)
        assertEquals(15, styles[0].start)
        assertEquals(20, styles[0].end)
        assertEquals("hallo", text.text.substring(styles[0].start, styles[0].end))
        // A cut line ends with "…"; a range past the text is clamped.
        val cut = hitText(ContentLine(line = 7, text = "abc", ranges = listOf(listOf(1, 99)), truncated = true), mark)
        assertEquals("Zeile 7: abc…", cut.text)
        assertEquals(10, cut.spanStyles[0].start)
        assertEquals(12, cut.spanStyles[0].end)
    }

    @Test
    fun content_search_task_skipped_files_and_poll_answer_decode() {
        val raw = """
            {"events":[
              {"type":"hits","rel":"a.txt","location":"/x/a.txt","size":5,"mtimeMs":1,"last":true,
               "lines":[{"line":1,"text":"Hallo","ranges":[[0,5]],"truncated":false,"context":false}]},
              {"type":"skipped","rel":"geheim","location":"/x/geheim","reason":"unlistable","detail":"Zugriff verweigert"},
              {"type":"skipped","rel":"bild.dat","location":"/x/bild.dat","reason":"binary","detail":null}
            ],"next":3,"files":4,"bytes":20,"hits":1,"done":true,"canceled":false,"error":null,
             "summary":{"filesSearched":4,"filesMatched":1,"hits":1,"bytes":20,"skipped":2,"limited":true}}
        """.trimIndent()
        val poll = Core.json.decodeFromString(ContentPoll.serializer(), raw)
        val results = ContentResults(ContentSearchQuery(pattern = "hallo"))
        results.apply(poll)
        assertEquals(
            listOf("geheim: Ordner nicht lesbar: Zugriff verweigert", "bild.dat: Binärdatei (keine Textdatei)"),
            results.skipped.map { "${it.rel}: ${it.reason}" },
        )
        assertTrue(results.limited)
        assertEquals(listOf(listOf(0, 5)), results.files[0].lines[0].ranges)
        assertEquals("nicht lesbar: kaputt", skipReason("unreadable", "kaputt"))
    }
}
