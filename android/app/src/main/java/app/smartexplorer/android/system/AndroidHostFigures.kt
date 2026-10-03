package app.smartexplorer.android.system

import android.content.Context
import android.os.Environment
import android.os.SystemClock
import android.os.storage.StorageManager
import android.util.Log
import app.smartexplorer.android.api.AnalyzeApi
import app.smartexplorer.android.core.Core
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** Shares only our successful StorageStats measurement of the actual primary
 * volume. Hosting uses this producer even when no local analysis was opened. */
internal object AndroidHostFigures {
    private val lock = Mutex()
    private var measuredAt: Long? = null
    suspend fun refresh(context: Context, force: Boolean = false) = lock.withLock {
        val now = SystemClock.elapsedRealtime()
        if (!force && measuredAt?.let { now - it < 5 * 60_000 } == true) return@withLock
        val storage = context.getSystemService(StorageManager::class.java) ?: return@withLock
        val volume = storage.storageVolumes.firstOrNull { it.isPrimary &&
            (it.state == Environment.MEDIA_MOUNTED || it.state == Environment.MEDIA_MOUNTED_READ_ONLY) } ?: return@withLock
        val root = volume.directory?.absolutePath ?: return@withLock
        try {
            val figures = StorageStatsAccess.figuresFor(context, root) ?: return@withLock
            Core.call("sys.platformTotals", buildJsonObject {
                put("volume", root)
                put("platform", AnalyzeApi.platformJson(figures.platform))
            })
            measuredAt = SystemClock.elapsedRealtime()
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            Log.w("SmartExplorerHost", "Eigene Plattformzahlen nicht aktualisiert", e)
        }
    }
}
