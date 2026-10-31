package app.smartexplorer.android.service

import app.smartexplorer.android.api.PendingRights
import app.smartexplorer.android.api.RightsTarget
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** The announcement of a connection that arose in the background and waits for its rights. */
class AbgleichTaskRightsNotifierTest {
    private fun pending(target: RightsTarget, name: String) = PendingRights(target, origin = "pairing", since = 1, name = name)

    @Test
    fun abgleichTaskOnlyNewWaitingConnectionsAreAnnouncedOnce() {
        val a = pending(RightsTarget.direct("d1", "k1"), "Laptop")
        val b = pending(RightsTarget.room("p1"), "Team")
        assertEquals(listOf(a, b), newlyPending(emptySet(), listOf(a, b)))
        assertEquals(listOf(b), newlyPending(setOf(a.target.key), listOf(a, b)))
        assertTrue(newlyPending(setOf(a.target.key, b.target.key), listOf(a, b)).isEmpty())
    }

    @Test
    fun abgleichTaskTheTextNamesTheConnectionsAndSaysTheyHaveNoRights() {
        val one = rightsPendingText(listOf(pending(RightsTarget.direct("d1", "k1"), "Laptop")))
        assertEquals("Neue Verbindung wartet auf Rechte", one.first)
        assertTrue(one.second.startsWith("Laptop – bis zur Bestätigung ohne Rechte"))
        val many = rightsPendingText((1..5).map { pending(RightsTarget.direct("d$it", "k"), "G$it") })
        assertEquals("5 Verbindungen warten auf Rechte", many.first)
        assertTrue(many.second.startsWith("G1, G2, G3 und 2 weitere"))
        val unnamed = rightsPendingText(listOf(pending(RightsTarget.room("p"), "")))
        assertTrue(unnamed.second.startsWith("Raum"))
    }
}
