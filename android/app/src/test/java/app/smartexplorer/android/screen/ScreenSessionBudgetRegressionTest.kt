package app.smartexplorer.android.screen

import app.smartexplorer.android.api.ScreenBufferBudget
import app.smartexplorer.android.core.CoreEvents
import app.smartexplorer.android.core.ScreenViewEvent
import app.smartexplorer.android.ui.screen.ScreenSessionState
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.*
import org.junit.Test

/** These cases are prepared for the single remote task suite. */
class ScreenSessionBudgetRegressionTest {
    private fun accepted(budget: String? = null): ScreenViewEvent {
        val field = budget?.let { ",\"bufferBudget\":$it" }.orEmpty()
        val body = """{"type":"screenView","session":1,"kind":"accepted","path":"direct","platform":"windows","authorization":{"view":"allow","control":"allow"}$field}"""
        return CoreEvents.decodeScreenView(Json.parseToJsonElement(body) as JsonObject) ?: error("missing event")
    }

    @Test
    fun batches_android_session_budget_small_legacy_receiver_keeps_its_scaler_bound() {
        val image = 128 * 128 * 4
        val record = image + 256
        val packet = record + 1024
        val working = 20L * image + 32L * (record + 1099) + packet - image
        val budget = ScreenBufferBudget(working, image, record, packet, record + 32,
            legacy = true, boundedLegacy = true)
        assertTrue(budget.workingBytes < 512L * 1024 * 1024)
        assertEquals(7L * image, budget.scalingBytes())
        assertEquals(budget.scalingBytes(), budget.copy(boundedLegacy = false).scalingBytes())
        assertEquals(image, budget.checkedBytes(128, 128))
        assertThrows(IllegalArgumentException::class.java) { budget.checkedBytes(129, 128) }
        assertThrows(IllegalArgumentException::class.java) { budget.copy(workingBytes = working - 1) }
    }

    @Test
    fun batches_android_session_budget_invalid_explicit_legacy_offer_has_no_512_mib_fallback() {
        val state = ScreenSessionState().apply(accepted("""{"workingBytes":1,"imageBytes":65536,"recordBytes":1024,"packetBytes":65536,"pullBytes":65568,"legacy":true,"boundedLegacy":true}"""))
        assertTrue(state.accepted)
        assertFalse(state.frameContract)
        assertNull(state.bufferBudget)
    }

    @Test
    fun batches_android_session_budget_old_acceptance_without_budget_keeps_legacy_contract() {
        val state = ScreenSessionState().apply(accepted())
        assertTrue(state.accepted)
        assertFalse(state.frameContract)
        assertSame(ScreenBufferBudget.LEGACY, state.bufferBudget)
    }
}
