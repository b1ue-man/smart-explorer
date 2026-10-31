package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * `sync.clearHashCache` (api.md §4.7): the answer decoded with `Core.json`, which the settings
 * row shows. The Rust answer carries the steps as well; the decoder ignores them.
 */
class AbgleichTaskHashCacheApiTest {
    @Test
    fun abgleichTaskNotClearedAnswerKeepsItsSummaryWithTheUnreachedWorker() {
        val answer = Core.json.decodeFromString(
            HashCacheClear.serializer(),
            """{"cleared":false,
               "summary":"Prüfsummen-Cache nicht vollständig geleert: diese App geleert; Hintergrunddienst nicht erreichbar: Verbindung abgelehnt",
               "steps":{"app":{"state":"cleared","detail":""},
                        "worker":{"state":"not_reached","detail":"Verbindung abgelehnt"}}}""",
        )
        assertFalse(answer.cleared)
        assertTrue(answer.summary.contains("Hintergrunddienst nicht erreichbar: Verbindung abgelehnt"))
    }

    @Test
    fun abgleichTaskClearedAnswerDecodesWithoutSteps() {
        val answer = Core.json.decodeFromString(
            HashCacheClear.serializer(),
            """{"cleared":true,"summary":"Prüfsummen-Cache geleert: diese App geleert; Hintergrunddienst läuft nicht"}""",
        )
        assertTrue(answer.cleared)
        assertEquals("Prüfsummen-Cache geleert: diese App geleert; Hintergrunddienst läuft nicht", answer.summary)
    }

    @Test
    fun abgleichTaskEmptyAnswerDefaultsToNotCleared() {
        val answer = Core.json.decodeFromString(HashCacheClear.serializer(), "{}")
        assertFalse(answer.cleared)
        assertEquals("", answer.summary)
    }

    @Test
    fun abgleichTaskBridgeMethodIsTheNameTheRustDispatcherRoutes() {
        assertEquals("sync.clearHashCache", SyncApi.CLEAR_HASH_CACHE_METHOD)
    }
}
