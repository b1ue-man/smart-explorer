package app.smartexplorer.android.launcher.model

import android.util.AtomicFile
import android.util.Log
import java.io.File
import java.io.FileNotFoundException
import java.io.IOException
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicReference

/**
 * `<filesDir>/launcher/layout.json` on disk (U35-18), owned by the `:launcher` process. Reads and writes
 * through [AtomicFile]; writes run on a single thread, and a [save] replaces one that has not begun
 * (the newest wins). [flush] finishes writing synchronously (`onStop` of the activity).
 */
class LayoutStore(private val dir: File) {
    private val file = File(dir, FILE)
    private val pending = AtomicReference<HomeLayout?>(null)
    private val lock = Any()
    private val executor = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "launcher-layout").apply { isDaemon = true }
    }

    fun load(): LayoutReadResult {
        val text = try {
            String(AtomicFile(file).readFully(), Charsets.UTF_8)
        } catch (_: FileNotFoundException) {
            return LayoutReadResult.Missing
        } catch (e: IOException) {
            return LayoutReadResult.Corrupt("nicht lesbar: ${e.message}")
        }
        return LayoutJson.parse(text)
    }

    /**
     * Sets a corrupt file aside as `layout.json.corrupt` (an older one is replaced, so at most one lies there)
     * and creates the empty marker `recovered` for the settings card (U35-18).
     */
    fun setAsideCorrupt() = synchronized(lock) {
        try {
            dir.mkdirs()
            val aside = File(dir, "$FILE.corrupt")
            aside.delete()
            if (!file.renameTo(aside)) {
                file.copyTo(aside, overwrite = true)
                file.delete()
            }
            File(dir, RECOVERED).createNewFile()
        } catch (e: IOException) {
            Log.w(TAG, "corrupt layout not set aside", e)
        }
    }

    /** Queues [layout] for writing; a layout queued before and not yet begun is dropped. */
    fun save(layout: HomeLayout) {
        pending.set(layout)
        executor.execute { writePending() }
    }

    /** Writes a queued layout now, on the calling thread. */
    fun flush() = writePending()

    private fun writePending() = synchronized(lock) {
        val layout = pending.getAndSet(null) ?: return@synchronized
        dir.mkdirs()
        val atomic = AtomicFile(file)
        val out = atomic.startWrite()
        try {
            out.write(LayoutJson.encode(layout).toByteArray(Charsets.UTF_8))
            atomic.finishWrite(out)
        } catch (e: IOException) {
            atomic.failWrite(out)
            Log.w(TAG, "layout.json not written", e)
        }
    }

    companion object {
        const val FILE = "layout.json"
        const val RECOVERED = "recovered"
        private const val TAG = "LayoutStore"
    }
}
