package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * Plan 2026-11-08 E49: `share.status` names an offer that ended after too many failed PIN attempts
 * and waits for its renewal; the phone shows the seconds until then and „Nicht erneuern“, which
 * stops [RenewalWaiting.replaces]. A core predating it sends no `renewalWaiting`.
 */
class PinNeuTaskRenewalWaitingTest {
    private fun decode(discovery: String): ShareDiscovery = Core.json.decodeFromString(
        ShareStatus.serializer(),
        """{"identity":{},"discovery":$discovery}""",
    ).discovery

    @Test
    fun pinNeuTaskRenewalWaitingIsReadWithItsStopTarget() {
        val discovery = decode(
            """{"offer":null,"renewalWaiting":{"replaces":"ersetzt","alias":"Pixel","dueMs":1700000030000,
               "hint":"5 falsche PIN-Versuche – vermutlich Rateversuche; neue PIN"},"advertisements":[]}""",
        )
        val waiting = discovery.renewalWaiting
        assertNotNull(waiting)
        assertEquals("ersetzt", waiting!!.replaces)
        assertEquals("Pixel", waiting.alias)
        assertEquals(1_700_000_030_000L, waiting.dueMs)
        assertEquals("5 falsche PIN-Versuche – vermutlich Rateversuche; neue PIN", waiting.hint)
        assertNull(discovery.offer)
    }

    @Test
    fun pinNeuTaskOlderCoreSendsNoRenewalWaiting() {
        assertNull(decode("""{"offer":null,"advertisements":[]}""").renewalWaiting)
    }
}
