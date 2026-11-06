package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The start decision of the documents worker from `docs.pending` (native `docs_pending.rs`:
 * `pending` is null while any cache is unknown, `knownPending` counts the readable ones, `unknown`
 * names the caches by root label, a vault as `vault:<site>`). A sealed vault cache alone waits for
 * the unlock; counted changes and other unknown caches are retried at the start.
 */
class DocsPendingStartTest {
    private fun decode(json: String): DocsPending = Core.json.decodeFromString(DocsPending.serializer(), json)

    @Test
    fun aSealedVaultAloneWaitsForItsUnlock() {
        val pending = decode("""{"pending":null,"knownPending":0,"unknown":["vault:abc"]}""")
        assertFalse(pending.isClean)
        assertFalse(pending.retryableNow)
        assertEquals(setOf("abc"), pending.sealedVaults)
    }

    @Test
    fun countedChangesAndOtherUnknownCachesAreRetriedNow() {
        assertTrue(decode("""{"pending":null,"knownPending":2,"unknown":["vault:abc"]}""").retryableNow)
        assertTrue(decode("""{"pending":null,"knownPending":0,"unknown":["gdrive"]}""").retryableNow)
        assertTrue(decode("""{"pending":3,"knownPending":3,"unknown":[]}""").retryableNow)
    }

    @Test
    fun theFingerprintIgnoresOrderAndThePerRunFailures() {
        val start = decode("""{"pending":null,"knownPending":1,"unknown":["gdrive","vault:abc"]}""")
        val run = decode("""{"pending":null,"knownPending":1,"unknown":["vault:abc","gdrive"],"failed":["gdrive"]}""")
        assertEquals(start.fingerprint, run.fingerprint)
        assertNotEquals(start.fingerprint, decode("""{"pending":2,"knownPending":2,"unknown":[]}""").fingerprint)
    }
}
