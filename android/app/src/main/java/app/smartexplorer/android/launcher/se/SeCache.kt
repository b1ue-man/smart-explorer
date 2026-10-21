package app.smartexplorer.android.launcher.se

import android.util.AtomicFile
import android.util.Log
import app.smartexplorer.android.launcher.bridge.BridgeProtocol
import app.smartexplorer.android.launcher.bridge.DevicesData
import app.smartexplorer.android.launcher.bridge.DevicesJson
import java.io.File
import java.io.IOException
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * `<filesDir>/launcher/se-cache.json` (DSK-M13, U40-8): the last `DEVICES` the launcher received, with `"stale": true`
 * and **without** the navigation token (the token lives in memory only). Owned by the process `:launcher`. Written at
 * most once per second, the newest message winning; read at start. A missing or unreadable file is an empty state.
 * The directory belongs to the launcher, so `LauncherSwitch.remove` deletes the file with it (U35-10).
 */
class SeCache(
    private val dir: File,
    private val scope: CoroutineScope,
    private val clock: () -> Long = System::currentTimeMillis,
) {
    private val file: File get() = File(dir, BridgeProtocol.SE_CACHE_FILE)
    private val latest = AtomicReference<DevicesData?>(null)
    private val lock = Any()
    private var job: Job? = null

    @Volatile
    private var lastWrite = 0L

    /** The stored message as stale data; `null` when there is none or it cannot be read. */
    fun read(): DevicesData? {
        if (!file.isFile) return null
        return try {
            DevicesJson.parse(file.readText(Charsets.UTF_8))?.takeIf { it.error == null }?.copy(navToken = null, stale = true)
        } catch (e: IOException) {
            Log.w(TAG, "se-cache.json unreadable", e)
            null
        }
    }

    /** Queues [data] for writing; a message that arrives within a second of the last write replaces an earlier queued one. */
    fun write(data: DevicesData) {
        if (data.error != null) return
        latest.set(data)
        synchronized(lock) {
            if (job?.isActive == true) return
            job = scope.launch(Dispatchers.IO) { flushLoop() }
        }
    }

    private suspend fun flushLoop() {
        var wait = (lastWrite + MIN_INTERVAL_MS - clock()).coerceAtLeast(0)
        while (true) {
            if (wait > 0) delay(wait)
            latest.getAndSet(null)?.let {
                store(it)
                lastWrite = clock()
            }
            synchronized(lock) {
                if (latest.get() == null) {
                    job = null
                    return
                }
            }
            wait = MIN_INTERVAL_MS
        }
    }

    private fun store(data: DevicesData) {
        val atomic = AtomicFile(file)
        try {
            if (!dir.isDirectory && !dir.mkdirs() && !dir.isDirectory) throw IOException("Ordner nicht anlegbar: ${dir.path}")
            val out = atomic.startWrite()
            try {
                out.write(DevicesJson.encode(data, withToken = false).toByteArray(Charsets.UTF_8))
                atomic.finishWrite(out)
            } catch (e: IOException) {
                atomic.failWrite(out)
                throw e
            }
        } catch (e: IOException) {
            Log.w(TAG, "se-cache.json not written", e)
        }
    }

    private companion object {
        const val TAG = "SeCache"

        /** At most one write per second (U40-8). */
        const val MIN_INTERVAL_MS = 1_000L
    }
}
