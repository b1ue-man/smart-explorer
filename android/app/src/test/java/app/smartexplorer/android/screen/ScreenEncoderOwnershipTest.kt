package app.smartexplorer.android.screen

import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/** Uses the production ownership state machines; no JNI or mocked MediaCodec lifetime. */
class ScreenEncoderOwnershipTest {
    private class Resource(val name: String, val closed: AtomicInteger = AtomicInteger()) : AutoCloseable {
        override fun close() { closed.incrementAndGet() }
    }

    @Test fun failedConstructionReturnsEveryAcquiredResourceInOrder() {
        for (stage in 0..3) {
            val released=mutableListOf<String>()
            val construction=ScreenCaptureConstruction<Resource,Resource>({released+=it.name},{released+=it.name})
            val failure=OutOfMemoryError("construction stage $stage")
            if(stage>=1) construction.ownClaim(ScreenCaptureLease(51L) {released+="claim:$it"})
            if(stage>=2) construction.ownProducer(Resource("codec"))
            if(stage>=3) construction.ownSurface(Resource("surface"))
            assertSame(failure,assertThrows(OutOfMemoryError::class.java) {construction.abort(failure)})
            val expected=when(stage) {
                0 -> emptyList()
                1 -> listOf("claim:51")
                2 -> listOf("codec","claim:51")
                else -> listOf("codec","surface","claim:51")
            }
            assertEquals(expected,released)
            assertFalse(construction.enterRuntime())
            assertSame(failure,assertThrows(OutOfMemoryError::class.java) {construction.abort(failure)})
            assertEquals(expected,released)
        }
    }

    @Test fun rejectedCodecCanBeReplacedWithoutReleasingTheSurfaceClaim() {
        val released=mutableListOf<String>()
        val construction=ScreenCaptureConstruction<Resource,Resource>({released+=it.name},{released+=it.name})
        construction.ownClaim(ScreenCaptureLease(52L) {released+="claim:$it"})
        construction.ownProducer(Resource("rejected"))
        construction.discardProducer()
        assertEquals(listOf("rejected"),released)
        construction.ownProducer(Resource("accepted"))
        val failure=IllegalStateException("thread start fixture")
        assertSame(failure,assertThrows(IllegalStateException::class.java) {construction.abort(failure)})
        assertEquals(listOf("rejected","accepted","claim:52"),released)
    }

    @Test fun failedPlatformDestructionKeepsTheClaimAndTheOriginalFailure() {
        val released=mutableListOf<String>()
        val cleanup=IllegalStateException("codec release fixture")
        val construction=ScreenCaptureConstruction<Resource,Resource>({throw cleanup},{released+=it.name})
        construction.ownClaim(ScreenCaptureLease(53L) {released+="claim:$it"})
        construction.ownProducer(Resource("codec"))
        construction.ownSurface(Resource("surface"))
        val failure=OutOfMemoryError("thread creation fixture")
        assertSame(failure,assertThrows(OutOfMemoryError::class.java) {construction.abort(failure)})
        assertEquals(emptyList<String>(),released)
        assertEquals(listOf(cleanup),failure.suppressed.toList())
        assertFalse(construction.enterRuntime())
    }

    @Test fun abortedConstructionRejectsALateDrainBeforeAnyResourceAccess() {
        val released=mutableListOf<String>()
        val used=AtomicInteger()
        val construction=ScreenCaptureConstruction<Resource,Resource>({released+=it.name},{released+=it.name})
        construction.ownClaim(ScreenCaptureLease(54L) {released+="claim:$it"})
        construction.ownProducer(Resource("codec"))
        construction.ownSurface(Resource("surface"))
        val late=Thread({if(construction.enterRuntime()) used.incrementAndGet()},"capture-late-drain-fixture").apply {isDaemon=true}
        assertThrows(OutOfMemoryError::class.java) {construction.abort(OutOfMemoryError("start fixture"))}
        late.start()
        late.join(2_000)
        assertFalse(late.isAlive)
        assertEquals(0,used.get())
        assertEquals(listOf("codec","surface","claim:54"),released)
    }

    @Test fun enteredDrainKeepsItsResourcesUntilItsActualEnd() {
        val released=mutableListOf<String>()
        val claim=ScreenCaptureLease(55L) {released+="claim:$it"}
        val codec=Resource("codec");val surface=Resource("surface")
        val construction=ScreenCaptureConstruction<Resource,Resource>({released+=it.name},{released+=it.name})
        construction.ownClaim(claim);construction.ownProducer(codec);construction.ownSurface(surface)
        val entered=CountDownLatch(1);val finish=CountDownLatch(1)
        val worker=Thread({
            if(construction.enterRuntime()) {
                entered.countDown()
                try {finish.await()} finally {released+=codec.name;released+=surface.name;claim.close()}
            }
        },"capture-entered-drain-fixture").apply {isDaemon=true}
        worker.start()
        try {
            await(entered)
            val failure=OutOfMemoryError("post-entry fixture")
            assertSame(failure,assertThrows(OutOfMemoryError::class.java) {construction.abort(failure)})
            assertTrue(worker.isAlive)
            assertEquals(emptyList<String>(),released)
        } finally {finish.countDown();worker.join(2_000)}
        assertFalse(worker.isAlive)
        assertEquals(listOf("codec","surface","claim:55"),released)
    }

    @Test fun failedProducerWrapperDoesNotPublishAUserOrLoseAnAlias() {
        val released=mutableListOf<Long>()
        val root=ScreenCaptureLease(56L) {released+=it}
        val failure=OutOfMemoryError("completion wrapper fixture")
        val owners=ScreenSurfaceOwners {throw failure}
        assertSame(failure,assertThrows(OutOfMemoryError::class.java) {owners.retain(root) {true}})
        owners.beginRelease();owners.awaitEnded();root.close()
        assertEquals(listOf(56L),released)
    }

    @Test fun creatorRollbackNeedsNoJoinOfTheStillRunningCreator() {
        val released=mutableListOf<Long>()
        val root=ScreenCaptureLease(57L) {released+=it}
        val owners=ScreenSurfaceOwners()
        val first=owners.retain(root) {true}
        val second=owners.retain(root) {true}
        first.close();first.close()
        assertThrows(IllegalStateException::class.java) {owners.beginRelease()}
        assertEquals(emptyList<Long>(),released)
        second.close();owners.beginRelease();owners.awaitEnded();root.close()
        assertEquals(listOf(57L),released)
        assertThrows(IllegalStateException::class.java) {owners.retain(root) {true}}
    }

    @Test fun completedLeaseStillWaitsForTheClosingWorkerToReturn() {
        val released=AtomicInteger()
        val root=ScreenCaptureLease(58L) {released.incrementAndGet()}
        val owners=ScreenSurfaceOwners()
        val completion=owners.retain(root) {true}
        val closed=CountDownLatch(1);val returnNow=CountDownLatch(1)
        val waiting=CountDownLatch(1);val retired=CountDownLatch(1)
        val worker=Thread({completion.close();closed.countDown();returnNow.await()},"capture-producer-fixture").apply {isDaemon=true}
        val drain=Thread({waiting.countDown();owners.awaitEnded();root.close();retired.countDown()},"capture-join-fixture").apply {isDaemon=true}
        worker.start()
        try {
            await(closed)
            assertThrows(IllegalStateException::class.java) {owners.beginRelease()}
            drain.start();await(waiting)
            assertFalse(retired.await(100,TimeUnit.MILLISECONDS))
            // The join must not detach the list before checking actual thread end.
            assertThrows(IllegalStateException::class.java) {owners.beginRelease()}
            assertEquals(0,released.get())
        } finally {returnNow.countDown();worker.join(2_000);drain.join(2_000)}
        assertFalse(worker.isAlive);assertFalse(drain.isAlive)
        await(retired)
        assertEquals(1,released.get())
    }

    @Test fun interruptedDrainStillWaitsForTheProducerAndRestoresItsFlag() {
        val released=AtomicInteger()
        val restored=AtomicBoolean()
        val root=ScreenCaptureLease(59L) {released.incrementAndGet()}
        val owners=ScreenSurfaceOwners()
        val completion=owners.retain(root) {true}
        val waiting=CountDownLatch(1);val closeNow=CountDownLatch(1)
        val worker=Thread({closeNow.await();completion.close()},"capture-interrupted-producer-fixture").apply {isDaemon=true}
        val drain=Thread({
            Thread.currentThread().interrupt();waiting.countDown();owners.awaitEnded()
            restored.set(Thread.currentThread().isInterrupted);root.close()
        },"capture-interrupted-drain-fixture").apply {isDaemon=true}
        worker.start();drain.start()
        try {await(waiting);assertEquals(0,released.get())}
        finally {closeNow.countDown();worker.join(2_000);drain.join(2_000)}
        assertFalse(worker.isAlive);assertFalse(drain.isAlive)
        assertTrue(restored.get());assertEquals(1,released.get())
    }

    @Test fun projectionRollbackEndsItsStartedWorkerBeforeTheReaderAndClaim() {
        val released=mutableListOf<String>()
        val started=CountDownLatch(1);val stop=CountDownLatch(1)
        val worker=Thread({started.countDown();stop.await()},"projection-startup-fixture").apply {isDaemon=true}
        val construction=ScreenCaptureConstruction<Thread,Resource>({
            stop.countDown();it.join(2_000);check(!it.isAlive);released+="worker"
        },{released+=it.name})
        construction.ownClaim(ScreenCaptureLease(60L) {released+="claim:$it"})
        construction.ownSurface(Resource("reader"))
        construction.ownProducer(worker)
        worker.start()
        try {
            await(started)
            assertThrows(OutOfMemoryError::class.java) {construction.abort(OutOfMemoryError("handler wrapper fixture"))}
        } finally {stop.countDown();worker.join(2_000)}
        assertFalse(worker.isAlive)
        assertEquals(listOf("worker","reader","claim:60"),released)
    }

    @Test fun failedPixelViewConstructionReturnsItsActualAllocationOwner() {
        val slot=ScreenCaptureSlot<Resource>()
        val owner=Resource("partial")
        val failure=OutOfMemoryError("buffer view fixture")
        assertSame(failure,assertThrows(OutOfMemoryError::class.java) {
            slot.allocate({owner}) {throw failure}
        })
        assertEquals(1,owner.closed.get())
        slot.clear()
        assertEquals(1,owner.closed.get())
    }

    @Test fun producerEndBeforeStartupTransferLeavesTheConstructorResponsible() {
        val released=mutableListOf<String>()
        val construction=ScreenCaptureConstruction<Resource,Resource>({released+=it.name},{released+=it.name})
        construction.ownClaim(ScreenCaptureLease(61L) {released+="claim:$it"})
        construction.ownSurface(Resource("reader"));construction.ownProducer(Resource("worker"))
        assertFalse(construction.producerEnded())
        assertFalse(construction.enterRuntime())
        assertThrows(IllegalStateException::class.java) {construction.abort(IllegalStateException("early end fixture"))}
        assertEquals(listOf("worker","reader","claim:61"),released)
    }

    @Test fun failedAccessUnitDescriptorLeavesItsOwnerForTeardown() {
        val slot=ScreenCaptureSlot<Resource>()
        val owner=Resource("partial")
        slot.allocate({owner}) {Unit}
        val failure=OutOfMemoryError("AU descriptor fixture")
        assertSame(failure,assertThrows(OutOfMemoryError::class.java) {slot.transfer<Any> {throw failure}})
        assertEquals(0,owner.closed.get())
        slot.clear()
        assertEquals(1,owner.closed.get())
    }

    @Test fun completedAccessUnitTransfersItsExistingOwnerUntilConsumerEnd() {
        val slot=ScreenCaptureSlot<Resource>()
        val owner=Resource("partial")
        slot.allocate({owner}) {Unit}
        val output=slot.transfer {checkNotNull(it)}
        slot.clear()
        assertSame(owner,output)
        assertEquals(0,owner.closed.get())
        output.close()
        assertEquals(1,owner.closed.get())
    }

    @Test fun hybridQueueFailureClearsTheExactPendingInputAndAllowsRetry() {
        val pending=ScreenCaptureInputs<Resource>()
        val pixels=Resource("first")
        val input=ScreenCaptureInput(7L,101L) {pixels}
        pending.expect(input)
        assertTrue(pending.isPending())
        assertSame(input,pending.peek(7L))
        assertEquals(null,pending.peek(8L))
        assertTrue(pending.cancel(input))
        input.close();input.close()
        assertFalse(pending.isPending())
        assertEquals(1,pixels.closed.get())
        val retriedPixels=Resource("retry")
        val retried=ScreenCaptureInput(8L,102L) {retriedPixels}
        pending.expect(retried)
        assertSame(retried,pending.take(8L))
        pending.clear()
        assertEquals(0,retriedPixels.closed.get())
        retried.close()
        assertEquals(1,retriedPixels.closed.get())
    }

    @Test fun queueAbortCannotCloseAnInputAlreadyOwnedByTheOutputConsumer() {
        val pending=ScreenCaptureInputs<Resource>()
        val pixels=Resource("taken")
        val input=ScreenCaptureInput(9L,103L) {pixels}
        pending.expect(input)
        val consumer=checkNotNull(pending.take(9L))
        assertFalse(pending.cancel(input))
        pending.clear()
        assertEquals(0,pixels.closed.get())
        consumer.close()
        assertEquals(1,pixels.closed.get())
    }

    @Test fun hybridInputRetainFailureNeverPublishesAReplacement() {
        val pending=ScreenCaptureInputs<Resource>()
        val failure=OutOfMemoryError("input owner fixture")
        assertSame(failure,assertThrows(OutOfMemoryError::class.java) {
            ScreenCaptureInput<Resource>(10L,104L) {throw failure}
        })
        assertFalse(pending.isPending())
    }

    private fun await(latch: CountDownLatch) = assertTrue("Fixture thread did not reach its ownership boundary",latch.await(2,TimeUnit.SECONDS))
}
