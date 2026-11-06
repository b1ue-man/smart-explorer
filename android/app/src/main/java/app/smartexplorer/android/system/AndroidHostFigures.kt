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
 * volume. Hosting uses this producer even when no local analysis was opened.
 *
 * With usage access one measurement asks Android for the storage of every installed package
 * (`queryStatsForPackage` and its label each). While the UI is visible it repeats after
 * [VISIBLE_INTERVAL_MS]; in the background only after [BACKGROUND_INTERVAL_MS], and at once on
 * a volume event and when the UI becomes visible ([force]), and only while Share runs: in the
 * background the figures serve only another device's analysis of this phone (the local analysis
 * measures its own when it starts). The core keeps the figures with their
 * measurement time, which a remote analysis shows; it replaces them as a whole
 * (`remember_platform_totals`), so the volume figure cannot be refreshed on its own without
 * dropping the app list. */
internal object AndroidHostFigures {
    private const val VISIBLE_INTERVAL_MS = 5 * 60_000L

    /** Four measurements a day at most while nobody looks at this phone. */
    private const val BACKGROUND_INTERVAL_MS = 6 * 60 * 60_000L

    private val lock = Mutex()
    private var measuredAt: Long? = null
    suspend fun refresh(context: Context, force: Boolean = false) = lock.withLock {
        val now = SystemClock.elapsedRealtime()
        val visible = HostMonitor.uiVisible
        // In the background only a remote analysis through Share reads the figures.
        if (!force && !visible && !HostMonitor.share.value.online) return@withLock
        val interval = if (visible) VISIBLE_INTERVAL_MS else BACKGROUND_INTERVAL_MS
        if (!force && measuredAt?.let { now - it < interval } == true) return@withLock
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
