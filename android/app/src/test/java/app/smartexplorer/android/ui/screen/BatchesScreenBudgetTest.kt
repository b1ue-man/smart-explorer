package app.smartexplorer.android.ui.screen

import app.smartexplorer.android.api.ScreenBufferBudget
import java.nio.ByteOrder
import org.junit.Assert.*
import org.junit.Test

class BatchesScreenBudgetTest {
    private fun budget(): ScreenBufferBudget {
        val image = 64 * 1024
        val record = 1024
        val overhead = 1099
        val working = maxOf(24L * image, 20L * image + 32L * (record + overhead))
        return ScreenBufferBudget(working, image, record, image, image + 32)
    }

    @Test
    fun batches_android_budget_finances_twenty_image_owners_and_record_slots() {
        val b = budget()
        assertEquals(b.workingBytes - 13L * b.imageBytes - 32L * (b.recordBytes + b.recordOverheadBytes), b.scalingBytes())
        assertTrue(b.scalingBytes() >= 7L * b.imageBytes)
        assertEquals(b.imageBytes, b.checkedBytes(128, 128))
        assertThrows(IllegalArgumentException::class.java) { b.checkedBytes(129, 128) }
        assertThrows(IllegalArgumentException::class.java) { b.copy(workingBytes = 24L * b.imageBytes - 1) }
        // Keep the 24I floor satisfied while exhausting the separately funded record slots.
        assertThrows(IllegalArgumentException::class.java) { b.copy(recordBytes = b.imageBytes) }
        // A small legacy receiver keeps its scaler bound (the former batches_android_session_budget_small_legacy_receiver_keeps_its_scaler_bound).
        val legacyImage = 128 * 128 * 4
        val legacyRecord = legacyImage + 256
        val legacyPacket = legacyRecord + 1024
        val legacyWorking = 20L * legacyImage + 32L * (legacyRecord + 1099) + legacyPacket - legacyImage
        val legacyBudget = ScreenBufferBudget(
            legacyWorking, legacyImage, legacyRecord, legacyPacket, legacyRecord + 32,
            legacy = true, boundedLegacy = true,
        )
        assertTrue(legacyBudget.workingBytes < 512L * 1024 * 1024)
        assertEquals(7L * legacyImage, legacyBudget.scalingBytes())
        assertEquals(legacyBudget.scalingBytes(), legacyBudget.copy(boundedLegacy = false).scalingBytes())
        assertEquals(legacyImage, legacyBudget.checkedBytes(128, 128))
        assertThrows(IllegalArgumentException::class.java) { legacyBudget.checkedBytes(129, 128) }
        assertThrows(IllegalArgumentException::class.java) { legacyBudget.copy(workingBytes = legacyWorking - 1) }
    }

    @Test
    fun batches_android_pull_waits_for_acceptance_and_transfers_the_same_allocation() {
        var accepted: ScreenBufferBudget? = null
        val previous = ScreenPullBuffers(offered = { accepted })
        assertFalse(previous.prepare())
        assertEquals(32, previous.buffer.capacity())
        accepted = budget()
        assertTrue(previous.prepare())
        val owned = previous.buffer
        assertTrue(owned.isDirect)
        assertEquals(ByteOrder.LITTLE_ENDIAN, owned.order())
        assertEquals(accepted!!.pullBytes, owned.capacity())
        assertSame(owned, previous.forPayload(accepted!!.imageBytes + 16))
        assertNull(previous.forPayload(accepted!!.imageBytes + 17))
        assertFalse(previous.acceptsPayload(accepted!!.recordBytes + 1, false))
        assertTrue(previous.acceptsPayload(accepted!!.imageBytes, true))
        val next = ScreenPullBuffers(offered = { accepted })
        next.takeFrom(previous)
        assertSame(owned, next.buffer)
        assertEquals(32, previous.buffer.capacity())
        assertTrue(next.prepare())
        assertSame(owned, next.buffer)
        accepted = accepted!!.copy(workingBytes = accepted!!.workingBytes + 4096)
        assertThrows(IllegalArgumentException::class.java) { next.prepare() }
    }
}
