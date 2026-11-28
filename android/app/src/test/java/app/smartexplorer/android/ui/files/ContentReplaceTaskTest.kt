package app.smartexplorer.android.ui.files

import app.smartexplorer.android.api.ContentEvent
import app.smartexplorer.android.api.ContentLine
import app.smartexplorer.android.api.ContentPoll
import app.smartexplorer.android.api.ContentSearchQuery
import app.smartexplorer.android.api.ReplaceHit
import app.smartexplorer.android.api.ReplacePoll
import app.smartexplorer.android.api.UndoResult
import app.smartexplorer.android.api.UndoSkipped
import app.smartexplorer.android.core.Core
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * "In Dateien ersetzen" (api.md §4.14 `search.content.replace.*`): the hit lines a selection
 * takes, "Aus dem Ergebnis entfernen", the confirmation text and the outcome built from
 * `search.content.replace.poll` answers.
 */
class ContentReplaceTaskTest {
    private fun hit(line: Long, text: String, vararg ranges: Pair<Int, Int>, context: Boolean = false, truncated: Boolean = false) =
        ContentLine(line = line, text = text, ranges = ranges.map { listOf(it.first, it.second) }, truncated = truncated, context = context)

    private fun hits(rel: String, vararg lines: ContentLine, size: Long = 10, mtime: Long = 1_700) = ContentEvent(
        type = "hits", rel = rel, location = "/storage/emulated/0/P/$rel", size = size, mtimeMs = mtime, lines = lines.toList(), last = true,
    )

    private fun sample(): ContentResults {
        val results = ContentResults(ContentSearchQuery(pattern = "foo", filters = "*.txt"), "/storage/emulated/0/P")
        results.apply(
            ContentPoll(
                events = listOf(
                    hits("a.txt", hit(1, "foo", 0 to 3), hit(2, "davor", context = true), hit(3, "foo foo", 0 to 3, 4 to 7), size = 20, mtime = 5),
                    hits("b.txt", hit(9, "…lang foo…", 6 to 9, truncated = true)),
                    ContentEvent(type = "skipped", rel = "bild.dat", reason = "binary"),
                ),
                done = true,
            ),
        )
        return results
    }

    @Test
    fun replace_ui_task_hit_refs_take_selected_lines_and_files_without_context() {
        val results = sample()
        assertTrue(results.replaceable)
        // A line row: only that line; a file row: every hit line of the file, context lines passed over.
        val one = results.hitRefs(setOf("l:a.txt:2"))
        assertEquals(listOf(ReplaceHit("a.txt", 3, "foo foo", listOf(listOf(0, 3), listOf(4, 7)), false, 20, 5)), one)
        val file = results.hitRefs(setOf("f:a.txt", "l:a.txt:1"))
        assertEquals(listOf(1L, 3L), file.map { it.line })
        // A cut line carries no ranges (they count from the cut): the core replaces each match of the line.
        val cut = results.hitRefs(setOf("l:b.txt:0")).single()
        assertTrue(cut.truncated)
        assertEquals(emptyList<List<Int>>(), cut.ranges)
        assertEquals(listOf("a.txt", "a.txt", "b.txt"), results.allHitRefs().map { it.rel })
        assertEquals(4, ReplaceJob("/x", results.query, "bar", results.allHitRefs()).chosenPlaces)
        assertEquals(null, ReplaceJob("/x", results.query, "bar", null).chosenPlaces)
    }

    @Test
    fun replace_ui_task_dismiss_removes_lines_files_and_skipped_entries() {
        val results = sample()
        assertTrue(results.dismiss(setOf("l:a.txt:0")))
        assertEquals(listOf(2L, 3L), results.files[0].lines.map { it.line })
        // A file whose last hit line goes leaves the result; its context lines do not keep it.
        assertTrue(results.dismiss(setOf("l:a.txt:1")))
        assertEquals(listOf("b.txt"), results.files.map { it.rel })
        assertTrue(results.dismiss(setOf("s")))
        assertEquals(0, results.skipped.size)
        assertFalse(results.dismiss(setOf("l:fehlt.txt:0")))
        assertEquals("Suche „foo“ (1 Treffer in 1 Datei von 0 durchsuchten) [Normal]", results.header())
        assertTrue(results.dismiss(setOf("f:b.txt")))
        assertFalse(results.replaceable)
        // Files a replacement changed leave the search they came from.
        val again = sample()
        again.dropFiles(listOf("a.txt"))
        assertEquals(listOf("b.txt"), again.files.map { it.rel })
    }

    @Test
    fun replace_ui_task_only_finished_full_searches_are_replaceable() {
        val running = ContentResults(ContentSearchQuery(pattern = "x"))
        running.apply(ContentPoll(events = listOf(hits("a.txt", hit(1, "x", 0 to 1)))))
        assertFalse("the search still runs", running.replaceable)
        val names = ContentResults(ContentSearchQuery(pattern = "x", filesOnly = true))
        names.apply(ContentPoll(events = listOf(hits("a.txt", hit(1, "x", 0 to 1))), done = true))
        assertFalse("a file-name search names one hit per file", names.replaceable)
        val failed = ContentResults(ContentSearchQuery(pattern = "x"))
        failed.apply(ContentPoll(events = listOf(hits("a.txt", hit(1, "x", 0 to 1))), done = true, error = "kaputt"))
        assertFalse(failed.replaceable)
    }

    @Test
    fun replace_ui_task_confirmation_names_folder_filter_places_and_files() {
        val query = ContentSearchQuery(pattern = "foo", filters = "", respectIgnoreFiles = true)
        val ready = ReplaceStage.Ready(filesChanged = 2, replacements = 5, skipped = listOf(SkippedFile("c.txt", "/P/c.txt", "schreibgeschützt")))
        assertEquals(
            listOf(
                "Wirklich alle Vorkommen ersetzen in:",
                "/P",
                "Für Dateitypen: *.*",
                "Ignorierdateien (.gitignore) werden beachtet.",
                "5 Stellen in 2 Dateien durch „bar“ ersetzen?",
                "1 übersprungen (Gründe unten).",
                "Vor dem Überschreiben wird jede Datei gesichert; „Rückgängig“ stellt sie wieder her.",
            ),
            confirmLines(ReplaceJob("/P", query, "bar", null), ready),
        )
        val chosen = confirmLines(
            ReplaceJob("/P", ContentSearchQuery(pattern = "foo", filters = "*.kt"), "", listOf(ReplaceHit("a.kt", 1))),
            ReplaceStage.Ready(filesChanged = 1, replacements = 1, skipped = emptyList()),
        )
        assertEquals("Wirklich die ausgewählten Treffer ersetzen in:", chosen[0])
        assertEquals("Für Dateitypen: *.kt", chosen[2])
        assertEquals("1 Stelle in 1 Datei löschen (Ersetzen durch nichts)?", chosen[3])
    }

    @Test
    fun replace_ui_task_outcome_counts_skips_and_offers_undo_from_poll_answers() {
        val raw = """
            {"events":[
              {"type":"file","rel":"a.txt","location":"/P/a.txt","replacements":3},
              {"type":"skipped","rel":"c.txt","location":"/P/c.txt","reason":"seit der Suche geändert"},
              {"type":"file","rel":"b.txt","location":"/P/b.txt","replacements":1}
            ],"next":3,"files":7,"replacements":4,"done":true,"canceled":false,"error":null,
             "summary":{"filesSearched":7,"filesChanged":2,"replacements":4,"skipped":1,"undoId":"r-1"}}
        """.trimIndent()
        val poll = Core.json.decodeFromString(ReplacePoll.serializer(), raw)
        val job = ReplaceJob("/P", ContentSearchQuery(pattern = "foo", matchCase = true), "bar", null)
        val outcome = ReplaceOutcome(job)
        outcome.apply(ReplacePoll(events = poll.events.take(1), files = 2))
        assertEquals("Ersetzen „foo“ durch „bar“ (läuft: 3 Ersetzungen in 1 Datei, 2 durchsucht) [Normal: Groß/Klein]", outcome.header())
        assertEquals(UndoStage.Unavailable, outcome.undo)
        outcome.apply(ReplacePoll(events = poll.events.drop(1), next = 3, done = true, summary = poll.summary))
        assertEquals("Ersetzen „foo“ durch „bar“ (4 Ersetzungen in 2 Dateien von 7 durchsuchten, 1 übersprungen) [Normal: Groß/Klein]", outcome.header())
        assertEquals(listOf("a.txt" to 3L, "b.txt" to 1L), outcome.changed)
        assertEquals(listOf("c.txt: seit der Suche geändert"), outcome.skipped.map { "${it.rel}: ${it.reason}" })
        assertEquals(UndoStage.Available("r-1"), outcome.undo)
        outcome.undo = UndoStage.Done
        assertTrue(outcome.header().endsWith(" – rückgängig gemacht"))
        outcome.undo = UndoStage.Partly(listOf(SkippedFile("a.txt", "", "nicht zurückgesetzt: seit dem Ersetzen geändert")))
        assertTrue(outcome.header().endsWith(" – teilweise rückgängig gemacht, 1 nicht zurückgesetzt"))

        val canceled = ReplaceOutcome(ReplaceJob("/P", ContentSearchQuery(pattern = "x"), "y", listOf(ReplaceHit("a.txt", 1))))
        canceled.apply(ReplacePoll(events = poll.events.take(1), done = true, canceled = true))
        assertEquals("Ersetzen (Auswahl) „x“ durch „y“ (abgebrochen: 3 Ersetzungen in 1 Datei, 0 übersprungen) [Normal]", canceled.header())
    }

    @Test
    fun replace_ui_task_hit_json_carries_the_contract_fields() {
        val encoded = Core.json.encodeToJsonElement(ReplaceHit.serializer(), ReplaceHit("a/b.txt", 4, "x y", listOf(listOf(2, 3)), false, 9, 77)).jsonObject
        assertEquals(setOf("rel", "line", "text", "ranges", "truncated", "size", "mtimeMs"), encoded.keys)
        val query = Core.json.encodeToJsonElement(ContentSearchQuery.serializer(), ContentSearchQuery(pattern = "x", respectIgnoreFiles = true)).jsonObject
        assertEquals("true", query["respectIgnoreFiles"].toString())
        val undo = Core.json.decodeFromString(
            UndoResult.serializer(),
            """{"restored":["a.txt"],"skipped":[{"rel":"b.txt","reason":"seit dem Ersetzen geändert"}]}""",
        )
        assertEquals(listOf("a.txt"), undo.restored)
        assertEquals(UndoSkipped("b.txt", "seit dem Ersetzen geändert"), undo.skipped.single())
    }
}
