package app.smartexplorer.android.screen

import java.util.Collections
import java.util.IdentityHashMap

/** Failed or delayed teardown retains the exact source/encoder pair. */
internal object ScreenSyntheticRetirement {
    private class Owner(val source: SyntheticSurfaceSource, val encoder: ScreenEncoder)
    private val retained = Collections.synchronizedMap(IdentityHashMap<Owner, Boolean>())

    /** The callback is success only after the producer worker and encoder drain ended. */
    fun retireSynthetic(source: SyntheticSurfaceSource, encoder: ScreenEncoder, onFinished: (Result<Unit>) -> Unit) {
        val owner = Owner(source, encoder)
        retained[owner] = true
        val finish = {
            val result = runCatching {
                check(source.awaitStopped(0)) { "Synthetic worker has not ended" }
                encoder.release()
            }
            if (result.isSuccess) retained.remove(owner)
            onFinished(result)
        }
        val stopped = try { source.stop() } catch (failure: Throwable) {
            onFinished(Result.failure(failure)); return
        }
        if (stopped) { finish(); return }
        try {
            Thread({ finish() }, "screen-synthetic-retirement").apply { isDaemon = true }.start()
        } catch (failure: Throwable) {
            // The map retains ownership if the retirement thread cannot start.
            onFinished(Result.failure(failure))
        }
    }
}
