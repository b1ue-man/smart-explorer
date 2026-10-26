package app.smartexplorer.android.ui.screen

import androidx.compose.ui.geometry.Rect
import app.smartexplorer.android.api.*
import app.smartexplorer.android.core.ScreenViewEvent
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = android.app.Application::class)
class BatchesScreenStateTest {
    private class Backend : ScreenBackend {
        val flow = MutableSharedFlow<ScreenViewEvent>(extraBufferCapacity = 32)
        override val events: Flow<ScreenViewEvent> = flow
        val presented = CopyOnWriteArrayList<Pair<Long, Long>>()
        val gop = (1L shl 40) + 7
        val minimum = (1L shl 42) + 9
        override fun offers() = emptyList<DecoderOffer>()
        override fun pictures(session: Long) = PictureSource { _, _ -> -1 }
        override suspend fun devices(roomId: String?) = emptyList<ScreenDevice>()
        override suspend fun probe(target: ScreenTarget) = error("unused probe")
        override suspend fun open(target: ScreenTarget, mode: String, source: String?, displayWidth: Int, displayHeight: Int, maxFps: Int, decoders: List<DecoderOffer>): Long {
            flow.emit(ScreenViewEvent.Accepted(7, "direct", "windows", "allow", "allow",
                buildJsonObject { putJsonObject("frameContract") { put("version", 2) } }))
            flow.emit(ScreenViewEvent.Stream(7, "tiles", null, 64, 64, gop, "fixture", false))
            return 7
        }
        override suspend fun setMode(session: Long, mode: String, source: String?, width: Int?, height: Int?, fps: Int?, sharpness: Boolean, lossless: Boolean) = Unit
        override suspend fun input(session: Long, events: List<JsonObject>) = events.size
        override suspend fun control(session: Long, action: String) = Unit
        override suspend fun listWindows(session: Long) = Unit
        override suspend fun keyframe(session: Long) = Unit
        override suspend fun close(session: Long) = Unit
        override suspend fun visibility(session: Long, visibility: ScreenVisibilityReport) {
            flow.emit(ScreenViewEvent.VisibilityAck(session, visibility.geometryGeneration, minimum))
        }
        override suspend fun presented(session: Long, gopId: Long, frameId: Long) { presented += gopId to frameId }
    }

    private suspend fun await(condition: () -> Boolean) = withTimeout(5000) {
        while (!condition()) delay(5)
    }

    @Test
    fun batches_android_only_current_presented_identity_unlocks_input() = runBlocking {
        val backend = Backend()
        val session = ScreenSession(ScreenTarget("direct", "fixture", null), backend)
        try {
            session.open("full", null, 64, 64, 30, emptyList())
            await { session.state.value.stream != null }
            val rect = Rect(0f, 0f, 64f, 64f)
            session.publishVisibility(ScreenVisibilityFacts(true, rect, rect, rect, 64, 64))
            val frame = ScreenPresentedFrame(7, backend.gop, backend.minimum, session.presentationEpoch())
            await { session.isDisplayable(frame) }
            assertFalse("An eligible decoded frame is not a presentation", session.state.value.inputReady)
            val rejected = listOf(frame.copy(session = 8), frame.copy(gopId = 7),
                frame.copy(frameId = backend.minimum - 1), frame.copy(epoch = frame.epoch - 1))
            for (stale in rejected) {
                assertFalse(session.isDisplayable(stale))
                session.confirmPresented(stale)
                assertFalse(session.state.value.inputReady)
            }
            assertTrue(backend.presented.isEmpty())
            session.confirmPresented(frame)
            await { backend.presented.size == 1 }
            assertEquals(backend.gop to backend.minimum, backend.presented.single())
            assertTrue(session.state.value.inputReady)
            session.confirmPresented(frame)
            // Barrier through the same command queue makes the duplicate assertion deterministic.
            val barrier = CompletableDeferred<Unit>()
            session.call { barrier.complete(Unit) }
            withTimeout(5000) { barrier.await() }
            assertEquals(1, backend.presented.size)
            session.invalidateDisplay()
            assertFalse(session.isDisplayable(frame))
            session.confirmPresented(frame)
            assertFalse(session.state.value.inputReady)
            assertTrue(session.beginBitmapSnapshot())
            assertFalse(session.beginBitmapSnapshot())
            session.endBitmapSnapshot()
            assertTrue(session.beginBitmapSnapshot())
            session.endBitmapSnapshot()
        } finally { session.close() }
        assertFalse(session.beginBitmapSnapshot())
    }

    @Test
    fun batches_android_stream_cache_does_not_alias_low_gop_bits() {
        val old = ScreenViewEvent.Stream(7, "tiles", null, 64, 64, 7, "fixture", false)
        val fresh = old.copy(gop = (1L shl 40) + 7, width = 128)
        val state = ScreenSessionState().apply(old).apply(fresh)
        assertEquals(2, state.streams.size)
        assertEquals(64, state.streams.getValue(7).width)
        assertEquals(128, state.streams.getValue(fresh.gop).width)
    }
}
