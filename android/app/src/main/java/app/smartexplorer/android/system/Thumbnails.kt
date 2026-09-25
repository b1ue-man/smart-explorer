package app.smartexplorer.android.system

import android.media.ThumbnailUtils
import android.os.CancellationSignal
import android.os.OperationCanceledException
import android.util.LruCache
import android.util.Size
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import java.io.File
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import kotlin.coroutines.coroutineContext

/**
 * Image previews for local files (spec F4) via `ThumbnailUtils.createImageThumbnail` (API 29,
 * android-apis.md §9.1). Decoding runs on the IO dispatcher, at most [PARALLEL] at a time;
 * results and failures are cached per path, modification time and size.
 */
object Thumbnails {
    private const val PARALLEL = 3
    private const val CACHE_KB = 12 * 1024
    private const val MAX_FAILED = 512

    private val permits = Semaphore(PARALLEL)
    private val failed = LinkedHashSet<String>()

    private val cache = object : LruCache<String, ImageBitmap>(CACHE_KB) {
        override fun sizeOf(key: String, value: ImageBitmap): Int = (value.width * value.height * 4 / 1024).coerceAtLeast(1)
    }

    /** Cached thumbnail without decoding, or `null`. */
    fun cached(path: String, mtimeMs: Long, sizePx: Int): ImageBitmap? = cache.get(key(path, mtimeMs, sizePx))

    /**
     * Thumbnail of the local image [path], or `null` when it cannot be decoded (not an image,
     * unreadable, removed). Cancelling the caller cancels the decode.
     */
    suspend fun load(path: String, mtimeMs: Long, sizePx: Int): ImageBitmap? {
        val key = key(path, mtimeMs, sizePx)
        cache.get(key)?.let { return it }
        if (synchronized(failed) { key in failed }) return null
        return permits.withPermit {
            cache.get(key) ?: withContext(Dispatchers.IO) { decode(path, sizePx) }.also { bitmap ->
                if (bitmap != null) {
                    cache.put(key, bitmap)
                } else if (coroutineContext[Job]?.isActive != false) {
                    markFailed(key)
                }
            }
        }
    }

    private suspend fun decode(path: String, sizePx: Int): ImageBitmap? {
        val signal = CancellationSignal()
        val handle = coroutineContext[Job]?.invokeOnCompletion { signal.cancel() }
        return try {
            ThumbnailUtils.createImageThumbnail(File(path), Size(sizePx, sizePx), signal).asImageBitmap()
        } catch (e: IOException) {
            null
        } catch (e: OperationCanceledException) {
            null
        } catch (e: IllegalArgumentException) {
            null
        } catch (e: SecurityException) {
            null
        } finally {
            handle?.dispose()
        }
    }

    private fun markFailed(key: String) {
        synchronized(failed) {
            failed += key
            if (failed.size > MAX_FAILED) failed.remove(failed.first())
        }
    }

    private fun key(path: String, mtimeMs: Long, sizePx: Int) = "$path|$mtimeMs|$sizePx"
}
