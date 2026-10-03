package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * RV1 FA1/FA2: the analysis answers carry the notes apart from the read problems and the
 * duplicate summary says how many candidates were really compared; older answers without the new
 * fields still decode.
 */
class ReviewTaskAnalyzeShapesTest {
    @Test
    fun issuesCarryNotesApartFromReadProblems() {
        val issues = Core.json.decodeFromString(
            AnalyzeIssues.serializer(),
            """{"count":0,"text":"","notes":["2 Bereiche von Android geschützt","Detailansicht ab /x zusammengefasst"],
               "protectedCount":0,"protectedText":""}""",
        )
        assertEquals(0, issues.count)
        assertEquals(2, issues.notes.size)
        assertEquals("2 Bereiche von Android geschützt", issues.notes.first())
        val older = Core.json.decodeFromString(AnalyzeIssues.serializer(), """{"count":1,"text":"/a: Fehler"}""")
        assertTrue(older.notes.isEmpty())
    }

    @Test
    fun duplicateSummaryNamesComparedCandidates() {
        val summary = Core.json.decodeFromString(
            ReclaimSummary.serializer(),
            """{"files":900,"bytes":4096,"candidates":300,"compared":120,"groups":7,"protectedCount":0,
               "protectedText":"","errorCount":0,"errorText":"","limit":null}""",
        )
        assertEquals(300L, summary.candidates)
        assertEquals(120L, summary.compared)
        assertEquals(0L, Core.json.decodeFromString(ReclaimSummary.serializer(), "{}").compared)
    }

    @Test
    fun hostFiguresAndContentBoundTrashRemainOptional() {
        val node = Core.json.decodeFromString(AnalyzeNode.serializer(),
            """{"name":"Daten","size":10,"isDir":true,"children":[],"volumeTotal":1000,"volumeFree":400,"remote":true}""")
        assertEquals(1000L, node.volumeTotal)
        assertEquals(400L, node.volumeFree)
        assertTrue(node.remote)
        val older = Core.json.decodeFromString(AnalyzeNode.serializer(),
            """{"name":"Daten","size":10,"isDir":true,"children":[]}""")
        assertEquals(null, older.volumeTotal)
        assertTrue(!older.remote)
        val summary = Core.json.decodeFromString(ReclaimSummary.serializer(),
            """{"remote":true,"canRecycle":false,"recycleNote":"Gegenstelle nicht erreichbar","files":12}""")
        assertTrue(summary.remote)
        assertTrue(!summary.canRecycle)
        assertEquals(12L, summary.files)
        assertTrue(!Core.json.decodeFromString(ReclaimSummary.serializer(), "{}").canRecycle)
        val group = Core.json.decodeFromString(DuplicateGroup.serializer(), """{"size":10,"items":[]}""")
        assertTrue(!group.contentVerified)
        val moved = Core.json.decodeFromString(RecycleResult.serializer(),
            """{"moved":["share://direct/pc/Daten/Kopie"]}""")
        assertEquals(listOf("share://direct/pc/Daten/Kopie"), moved.moved)
    }
}
