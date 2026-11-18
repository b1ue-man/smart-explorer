package app.smartexplorer.android.ui.analytics

import app.smartexplorer.android.api.AnalyzeChild
import app.smartexplorer.android.api.AnalyzeNode
import app.smartexplorer.android.api.AnalyzeOverview
import app.smartexplorer.android.api.DuplicateGroup
import app.smartexplorer.android.api.ReclaimSummary
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.ui.common.Format
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * "Größe" and "Belegt" of the storage analysis (2026-11-18): the page shows both, sorts and sizes
 * the tiles by "Belegt" by default, falls back to "Größe" where the result does not know the
 * occupied space and then says "Belegt: unbekannt"; an older core's answer still decodes.
 */
class BelegtTaskSizeMeasureTest {
    @Test
    fun belegt_task_display_shows_both_values_and_unknown() {
        val gb5 = 5L shl 30
        val mb100 = 100L shl 20
        val node = Core.json.decodeFromString(
            AnalyzeNode.serializer(),
            """{"name":"Daten","size":$gb5,"allocated":$mb100,"allocatedKnown":true,"sort":"allocated","isDir":true,
               "children":[{"name":"geloescht","size":$gb5,"allocated":$mb100,"isDir":true,"kind":"dir"},
                           {"name":"≈ Nicht einzeln erfasst","size":7,"kind":"rest"}]}""",
        )
        assertTrue(node.allocatedKnown)
        assertEquals("allocated", node.sort)
        assertEquals(mb100, nodeAllocated(node))
        val deleted = node.children.first()
        assertEquals(mb100, SizeMeasure.Allocated.of(deleted))
        assertEquals(gb5, SizeMeasure.Size.of(deleted))
        assertEquals("Größe ${Format.size(gb5)} · Belegt ${Format.size(mb100)}", rowSizes(deleted, known = true))
        // A row of Android's figures occupies what Android reports: one value.
        assertEquals(Format.size(7), rowSizes(node.children[1], known = true))
        assertEquals(7L, SizeMeasure.Allocated.of(node.children[1]))

        // Default "Belegt"; unknown occupied space means "Größe" and "Belegt: unbekannt".
        assertEquals(SizeMeasure.Allocated, SizeMeasure.parse(null))
        assertEquals(SizeMeasure.Size, SizeMeasure.parse("size"))
        assertEquals(SizeMeasure.Allocated, SizeMeasure.parse("blocks"))
        assertEquals(SizeMeasure.Allocated, SizeMeasure.effective(SizeMeasure.Allocated, allocatedKnown = true))
        assertEquals(SizeMeasure.Size, SizeMeasure.effective(SizeMeasure.Allocated, allocatedKnown = false))
        assertEquals("Größe ${Format.size(gb5)} · Belegt: unbekannt", sizesText(gb5, gb5, known = false))
        assertTrue(MEASURE_EXPLANATION.contains("nur online") && MEASURE_EXPLANATION.contains("Hardlinks"))

        // An older core sends neither field: "Belegt" equals "Größe" and is unknown.
        val older = Core.json.decodeFromString(
            AnalyzeNode.serializer(),
            """{"name":"Daten","size":10,"isDir":true,"children":[{"name":"a","size":10,"kind":"file"}]}""",
        )
        assertFalse(older.allocatedKnown)
        assertEquals("size", older.sort)
        assertEquals(10L, nodeAllocated(older))
        assertEquals(10L, older.children.first().allocatedBytes)
        assertEquals(10L, AnalyzeChild(name = "b", size = 10).allocatedBytes)

        // "wird frei" of duplicates is the occupied space; an older core: size per further copy.
        val group = Core.json.decodeFromString(
            DuplicateGroup.serializer(),
            """{"size":$gb5,"reclaimable":0,"items":[{"location":"/a"},{"location":"/b"}]}""",
        )
        assertEquals(0L, group.freedBytes)
        val olderGroup = Core.json.decodeFromString(
            DuplicateGroup.serializer(),
            """{"size":10,"items":[{"location":"/a"},{"location":"/b"},{"location":"/c"}]}""",
        )
        assertEquals(20L, olderGroup.freedBytes)
    }

    @Test
    fun analyse_task_overview_decodes_both_values_and_aggregate_note() {
        val overview = Core.json.decodeFromString(
            AnalyzeOverview.serializer(),
            """{"sort":"allocated","allocatedKnown":true,
               "largest":[{"path":"geloescht/film.mkv","size":5000,"allocated":100,"location":"/sdcard/geloescht"}],
               "types":[{"extension":"mkv","files":1,"size":5000,"allocated":100}],
               "aggregatedFiles":7,"aggregatedSize":700,"aggregatedAllocated":700,
               "aggregatedNote":"7 Dateien zusammengefasst (große Ordner): sie zählen in keinen Dateityp und in keine Liste."}""",
        )
        assertEquals(7L, overview.aggregatedFiles)
        assertEquals("/sdcard/geloescht", overview.largest.single().location)
        assertEquals("Größe ${Format.size(5000)} · Belegt ${Format.size(100)}", largestDetails(overview.largest.single(), known = true))
        assertEquals("1 Dateien · Größe ${Format.size(5000)} · Belegt: unbekannt", typeDetails(overview.types.single(), known = false))
        assertEquals(null, Core.json.decodeFromString(AnalyzeOverview.serializer(), "{}").aggregatedNote)
        // Duplicates: "Belegt" only where the search measured it; an older core: only a local search.
        assertEquals("${Format.size(10)} wird frei (belegt)", freedText(10, known = true))
        assertEquals("${Format.size(10)} wird frei (Größe; Belegt unbekannt)", freedText(10, known = false))
        val summary = { json: String -> Core.json.decodeFromString(ReclaimSummary.serializer(), json) }
        assertTrue(summary("""{"remote":true,"allocatedKnown":true}""").occupiedKnown)
        assertFalse(summary("""{"remote":false,"allocatedKnown":false}""").occupiedKnown)
        assertFalse(summary("""{"remote":true}""").occupiedKnown)
        assertTrue(summary("{}").occupiedKnown)

        // Live counters of a running analysis with "Belegt" (task snapshot `allocatedBytes`).
        val task = TaskInfo(id = "t", kind = "analyze", title = "", state = "running", doneBytes = 4096, doneItems = 3)
        assertEquals("3 Dateien · ${Format.size(4096)}", progressLine(task))
        assertEquals("3 Dateien · Größe ${Format.size(4096)} · Belegt ${Format.size(8192)}",
            progressLine(task.copy(allocatedBytes = 8192)))
        assertEquals("3 Dateien · Größe ${Format.size(4096)} · Belegt: unbekannt", progressLine(task.copy(allocatedBytes = 0)))

        // The analysis way and uncounted file systems come first; NTFS system files open nothing.
        val notes = listOf("2 Bereiche geschützt", "anderes Dateisystem, nicht gezählt: /proc", "Analyseweg: schneller NTFS-Weg (MFT)")
        assertEquals(listOf(notes[1], notes[2], notes[0]), orderedNotes(notes))
        val system = AnalyzeChild(name = "NTFS-Systemdateien", size = 1, kind = "file", systemFiles = true)
        assertFalse(system.tappable)
        assertTrue(AnalyzeChild(name = "a", isDir = true, kind = "dir").tappable)
    }
}
