package app.smartexplorer.android.screen

/** A native token has an owner only after the complete Kotlin wrapper exists. */
internal inline fun <T> adoptCaptureOwner(token: Long, release: (Long) -> Unit, create: () -> T): T =
    try { create() } catch (failure: Throwable) { release(token); throw failure }

/** Shared by real aliases; a failed factory never publishes another reference. */
internal class ScreenCaptureReferences(private val token: Long, private val release: (Long) -> Unit) {
    private var references = 1

    @Synchronized fun <T> retain(create: () -> T): T {
        check(references > 0)
        val next = Math.addExact(references, 1)
        val alias = create()
        references = next
        return alias
    }

    @Synchronized fun close() {
        check(references > 0)
        references--
        if (references == 0) release(token)
    }
}

/** Created before any resource; constructor abort and runtime entry have one winner. */
internal class ScreenCaptureConstruction<P : Any, S : Any>(
    private val retireProducer: (P) -> Unit,
    private val releaseSurface: (S) -> Unit,
) {
    private enum class Owner { CONSTRUCTOR, RUNTIME, ABORTED }
    private var owner = Owner.CONSTRUCTOR
    private var ended = false
    private var claim: ScreenCaptureLease? = null
    private var producer: P? = null
    private var surface: S? = null

    fun ownClaim(value: ScreenCaptureLease?): ScreenCaptureLease? { claim = value; return value }
    fun ownProducer(value: P): P { producer = value; return value }
    fun ownSurface(value: S?): S? { surface = value; return value }
    fun discardProducer() { producer?.let { retireProducer(it); producer = null } }

    @Synchronized fun enterRuntime(): Boolean {
        if (owner != Owner.CONSTRUCTOR || ended) return false
        owner = Owner.RUNTIME
        // The completed instance's fields now own the producer and its surface.
        claim = null; producer = null; surface = null
        return true
    }

    @Synchronized fun producerEnded(): Boolean { ended = true; return owner == Owner.RUNTIME }

    fun abort(failure: Throwable): Nothing {
        val rollback = synchronized(this) {
            if (owner == Owner.CONSTRUCTOR) { owner = Owner.ABORTED; true } else false
        }
        if (rollback) try {
            // The producer may still use its surface if retirement fails.
            producer?.let { retireProducer(it); producer = null }
            surface?.let { releaseSurface(it); surface = null }
            claim?.close(); claim = null
        } catch (cleanup: Throwable) {
            if (cleanup !== failure) try { failure.addSuppressed(cleanup) } catch (_: Throwable) { Unit }
        }
        throw failure
    }
}

/** One allocation owner stays reachable until its complete output wrapper exists. */
internal class ScreenCaptureSlot<T : AutoCloseable> {
    private var owner: T? = null

    fun <R> allocate(create: () -> T?, initialize: (T?) -> R): R {
        check(owner == null)
        val value = create()
        owner = value
        try { return initialize(value) } catch (failure: Throwable) { clear(); throw failure }
    }

    fun <R> transfer(create: (T?) -> R): R {
        val output = create(owner)
        owner = null
        return output
    }

    fun clear() { owner?.close(); owner = null }
}

/** Allocate this wrapper before retaining pixels; completion is idempotent. */
internal class ScreenCaptureInput<T : AutoCloseable>(val ptsUs: Long, val frameId: Long, retain: () -> T) : AutoCloseable {
    val pixels = retain()
    private var closed = false
    @Synchronized override fun close() { if (!closed) { closed = true; pixels.close() } }
}

/** The encoder already accepts exactly one pending hybrid capture. */
internal class ScreenCaptureInputs<T : AutoCloseable> {
    private var input: ScreenCaptureInput<T>? = null
    @Synchronized fun isPending(): Boolean = input != null
    @Synchronized fun expect(value: ScreenCaptureInput<T>) { check(input == null); input = value }
    @Synchronized fun peek(ptsUs: Long): ScreenCaptureInput<T>? = input?.takeIf { it.ptsUs == ptsUs }
    @Synchronized fun take(ptsUs: Long): ScreenCaptureInput<T>? {
        val value = peek(ptsUs) ?: return null
        input = null
        return value
    }
    @Synchronized fun cancel(value: ScreenCaptureInput<T>): Boolean {
        if (input !== value) return false
        input = null
        return true
    }
    @Synchronized fun clear() { input?.close(); input = null }
}

/** Registration allocates; producer completion and traversal only use existing nodes. */
internal class ScreenSurfaceOwners(
    private val completionLease: ((Long) -> Unit) -> ScreenCaptureLease = { ScreenCaptureLease(0L, it) },
) {
    private class Producer(
        val retained: ScreenCaptureLease,
        val creator: Thread,
        val next: Producer?,
    ) { var endedOn: Thread? = null }

    private val lock = java.lang.Object()
    private var first: Producer? = null
    private var users = 0
    private var finishing = false

    fun retain(lease: ScreenCaptureLease, accepting: () -> Boolean): ScreenCaptureLease = synchronized(lock) {
        check(!finishing && accepting()) { "Encoder nimmt keinen neuen Surfaceproduzenten an" }
        val count = Math.addExact(users, 1)
        val retained = lease.retain()
        try {
            val producer = Producer(retained, Thread.currentThread(), first)
            val completion = completionLease {
                try { producer.retained.close() } finally { synchronized(lock) {
                    val endedOn = Thread.currentThread()
                    if (endedOn !== producer.creator) producer.endedOn = endedOn
                    users--
                    lock.notifyAll()
                } }
            }
            first = producer
            users = count
            completion
        } catch (failure: Throwable) { retained.close(); throw failure }
    }

    fun beginRelease() = synchronized(lock) {
        check(users == 0) { "Surfaceproduzent lebt noch; Encoder und Claims bleiben gehalten" }
        var producer = first
        while (producer != null) {
            check(producer.endedOn?.isAlive != true) { "Surfaceproduzent lebt noch; Encoder und Claims bleiben gehalten" }
            producer = producer.next
        }
        finishing = true
    }

    fun awaitEnded() {
        var interrupted = false
        var producer = synchronized(lock) {
            finishing = true
            while (users != 0) {
                try { lock.wait() } catch (_: InterruptedException) { interrupted = true }
            }
            first
        }
        // Keep first reachable during every join: beginRelease must still see
        // workers whose lease ended before their actual return from finally.
        while (producer != null) {
            val worker = producer.endedOn
            if (worker != null) {
                check(worker !== Thread.currentThread())
                while (worker.isAlive) {
                    try { worker.join() } catch (_: InterruptedException) { interrupted = true }
                }
            }
            producer = producer.next
        }
        if (interrupted) Thread.currentThread().interrupt()
    }
}
