package app.smartexplorer.android.screen

import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Assert.assertThrows
import org.junit.Test

/** Final-owner invariants without JNI, ImageReader or a device allocator. */
class ScreenCaptureBudgetTest {
    @Test fun callbackAndWorkerRetainTheSamePromiseUntilBothEnd() {
        val released=mutableListOf<Long>()
        val reader=ScreenCaptureLease(17L) {released+=it}
        val callback=reader.retain()
        val worker=reader.retain()
        reader.close()
        callback.close()
        assertEquals(emptyList<Long>(),released)
        worker.close()
        assertEquals(listOf(17L),released)
        worker.close()
        assertEquals(listOf(17L),released)
    }
    @Test fun aClosedHolderCannotCreateAReplacementOwner() {
        val released=mutableListOf<Long>()
        val holder=ScreenCaptureLease(9L) {released+=it}
        val callback=holder.retain()
        holder.close()
        assertThrows(IllegalStateException::class.java) {holder.retain()}
        assertEquals(emptyList<Long>(),released)
        callback.close()
        assertEquals(listOf(9L),released)
    }
    @Test fun failedTokenAdoptionReturnsTheRegisteredNativeTokenExactlyOnce() {
        val released=mutableListOf<Long>()
        val failure=OutOfMemoryError("owner wrapper fixture")
        val observed=assertThrows(OutOfMemoryError::class.java) {
            adoptCaptureOwner(41L,{released+=it}) {throw failure}
        }
        assertSame(failure,observed)
        assertEquals(listOf(41L),released)
    }
    @Test fun failedAliasFactoryLeavesOnlyTheTwoActualOwners() {
        val released=mutableListOf<Long>()
        val references=ScreenCaptureReferences(42L) {released+=it}
        references.retain {Any()}
        val failure=OutOfMemoryError("alias wrapper fixture")
        assertSame(failure,assertThrows(OutOfMemoryError::class.java) {
            references.retain<Any> {throw failure}
        })
        references.close()
        assertEquals(emptyList<Long>(),released)
        references.close()
        assertEquals(listOf(42L),released)
    }
}
