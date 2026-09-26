package app.smartexplorer.android.service

import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkInfo
import androidx.work.WorkManager
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.system.HostMonitor
import app.smartexplorer.android.work.SyncWorker
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * Background mode (spec F17, `AppPrefs.bgMode`): "off" = no scheduled jobs (daemon sync flag
 * off, no periodic work); "periodic" = WorkManager wakes the app for a catch-up run; "persistent"
 * = [BackgroundService] (specialUse) keeps the process and the embedded daemon awake. The daemon
 * itself runs once per process and is never stopped for visibility (api.md §4.7).
 */
object BackgroundController {
    const val MODE_OFF = "off"
    const val MODE_PERIODIC = "periodic"
    const val MODE_PERSISTENT = "persistent"

    /** Unique name of the periodic catch-up work. */
    const val WORK_NAME = "background-catch-up"

    private const val TAG = "SmartExplorerBg"
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val syncFlagLock = Mutex()

    /** From `SmartExplorerApp.onCreate` (any process start: UI, worker, boot). */
    fun onAppStart(context: Context) {
        val app = context.applicationContext
        HostMonitor.start(app)
        scope.launch {
            try {
                SyncApi.ensureDaemon()
            } catch (e: CoreException) {
                Log.w(TAG, "bg.ensureDaemon failed: ${e.kind}: ${e.displayText()}")
            }
        }
        apply(app)
    }

    /** From `MainActivity.onStart/onStop`: host state, task service and the persistent service. */
    fun onUiVisible(context: Context, visible: Boolean) {
        val app = context.applicationContext
        HostMonitor.setForeground(visible)
        TaskKeeper.onUiVisible(app, visible)
        // While the UI is visible a foreground service may always start; repairs a failed boot start.
        if (visible && AppPrefs.bgMode.value == MODE_PERSISTENT) startBackgroundService(app)
    }

    /** Applies the current settings: periodic work (UPDATE policy), sync flag, persistent service. */
    fun apply(context: Context) {
        val app = context.applicationContext
        val mode = AppPrefs.bgMode.value
        scheduleWork(app, mode)
        scope.launch {
            // One call at a time, each with the mode current at that moment: the last one to run
            // always sends the newest setting, however overlapping calls are ordered.
            syncFlagLock.withLock {
                try {
                    SyncApi.setSyncEnabled(AppPrefs.bgMode.value != MODE_OFF)
                } catch (e: CoreException) {
                    Log.w(TAG, "bg.setSyncEnabled failed: ${e.kind}: ${e.displayText()}")
                }
            }
        }
        if (mode == MODE_PERSISTENT) startBackgroundService(app) else stopBackgroundService(app)
    }

    /**
     * BOOT_COMPLETED / MY_PACKAGE_REPLACED: only the specialUse service (persistent mode) and the
     * periodic work. A dataSync service must never start from BOOT_COMPLETED (android-apis.md §4.3).
     */
    internal fun onBoot(context: Context) {
        val app = context.applicationContext
        val mode = AppPrefs.bgMode.value
        scheduleWork(app, mode)
        if (mode == MODE_PERSISTENT) startBackgroundService(app)
    }

    /** Next planned run of the periodic work, `null` when none is enqueued. */
    fun nextRun(context: Context): Flow<Long?> =
        WorkManager.getInstance(context.applicationContext)
            .getWorkInfosForUniqueWorkFlow(WORK_NAME)
            .map { infos ->
                infos.firstOrNull { it.state == WorkInfo.State.ENQUEUED }
                    ?.nextScheduleTimeMillis
                    ?.takeIf { it > 0 && it != Long.MAX_VALUE }
            }

    /**
     * Periodic catch-up work in "periodic" and "persistent" mode; in "persistent" mode it is only
     * the fallback for a service the system ended ([SyncWorker] skips while the service runs).
     */
    private fun scheduleWork(context: Context, mode: String) {
        val workManager = WorkManager.getInstance(context)
        if (mode == MODE_OFF) {
            workManager.cancelUniqueWork(WORK_NAME)
            return
        }
        val constraints = Constraints.Builder()
            .setRequiredNetworkType(if (AppPrefs.bgWifiOnly.value) NetworkType.UNMETERED else NetworkType.NOT_REQUIRED)
            .setRequiresCharging(AppPrefs.bgChargingOnly.value)
            .setRequiresBatteryNotLow(AppPrefs.bgBatteryNotLow.value)
            .build()
        val request = PeriodicWorkRequestBuilder<SyncWorker>(AppPrefs.bgIntervalMin.value.toLong(), TimeUnit.MINUTES)
            .setConstraints(constraints)
            .build()
        workManager.enqueueUniquePeriodicWork(WORK_NAME, ExistingPeriodicWorkPolicy.UPDATE, request)
    }

    private fun startBackgroundService(context: Context) {
        if (BackgroundService.isRunning) return
        try {
            ContextCompat.startForegroundService(context, Intent(context, BackgroundService::class.java))
        } catch (e: IllegalStateException) {
            // ForegroundServiceStartNotAllowedException (API 31+) is an IllegalStateException:
            // started from the background without an exemption. The next UI start or boot retries.
            Log.w(TAG, "background service not started", e)
        } catch (e: SecurityException) {
            Log.w(TAG, "background service not started", e)
        }
    }

    private fun stopBackgroundService(context: Context) {
        context.stopService(Intent(context, BackgroundService::class.java))
    }
}
