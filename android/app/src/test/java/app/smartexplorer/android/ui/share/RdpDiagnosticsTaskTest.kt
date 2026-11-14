package app.smartexplorer.android.ui.share

import app.smartexplorer.android.api.RouteBudget
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Lines of the RDP diagnostics shown under the Remote Desktop state: every reported line stays, in order; a remote
 * device shows its own feed lines only (E54); the dropped-candidate line of the route budget (F13).
 */
class RdpDiagnosticsTaskTest {
    @Test
    fun rdpDiagnosticsTaskEmptyListShowsNothing() {
        assertEquals(emptyList<String>(), rdpDiagnosticLines(emptyList()))
    }

    @Test
    fun rdpDiagnosticsTaskKeepsOrderAndDropsOnlyBlankLines() {
        val reported = listOf("GNOME: Netz nicht erreichbar", "", "   ", "Port 3389 offen", "\t")
        assertEquals(listOf("GNOME: Netz nicht erreichbar", "Port 3389 offen"), rdpDiagnosticLines(reported))
    }

    @Test
    fun rdpDiagnosticsTaskKeepsEveryLineLiteral() {
        val long = "x".repeat(600) + " Ende"
        val spaced = "  führende Leerzeichen und Ä/ö/ß  "
        val reported = listOf(long, spaced, "Zeile mit \"Anführungszeichen\" und \\ Backslash")
        assertEquals(reported, rdpDiagnosticLines(reported))
    }

    @Test
    fun rdpDiagnosticsTaskForeignDeviceShowsOnlyItsOwnFeedLines() {
        val feed = listOf("GNOME: Netz nicht erreichbar")
        val phone = listOf("Telefon: Probe ok")
        assertEquals(feed, rdpDiagnosticSource(foreign = true, feedLines = feed, localLines = phone))
        assertEquals(emptyList<String>(), rdpDiagnosticSource(foreign = true, feedLines = emptyList(), localLines = phone))
    }

    @Test
    fun rdpDiagnosticsTaskOwnDeviceShowsLocalLines() {
        val phone = listOf("Telefon: Probe ok")
        assertEquals(phone, rdpDiagnosticSource(foreign = false, feedLines = listOf("fremd"), localLines = phone))
    }

    @Test
    fun rdpDiagnosticsTaskSelectsForeignDeviceByContactOrIdentity() {
        assertTrue(rdpSelectsForeignDevice(contactId = "kontakt", deviceId = null, publicKey = null))
        assertTrue(rdpSelectsForeignDevice(contactId = null, deviceId = "geraet", publicKey = "schluessel"))
        assertFalse(rdpSelectsForeignDevice(contactId = null, deviceId = "geraet", publicKey = null))
        assertFalse(rdpSelectsForeignDevice(contactId = null, deviceId = null, publicKey = null))
    }

    @Test
    fun rdpDiagnosticsTaskShowsDroppedCandidatesOnlyWhenCounted() {
        assertEquals("Verworfene Kandidaten über der Grenze: 7", droppedCandidatesLine(routeBudget(dropped = 7)))
        assertNull(droppedCandidatesLine(routeBudget(dropped = null)))
    }

    private fun routeBudget(dropped: Int?) = RouteBudget(
        initialPtoMs = 300, bestPtoMs = 120, collectionMs = 500, retryIntervalsMs = listOf(100, 200),
        responseMs = 1_000, validationMs = 2_000, frozenCandidates = 128, droppedCandidates = dropped,
    )
}
