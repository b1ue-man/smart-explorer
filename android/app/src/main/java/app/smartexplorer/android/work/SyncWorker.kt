package app.smartexplorer.android.work

import android.content.Context
import android.content.pm.ServiceInfo
import android.util.Log
import androidx.work.CoroutineWorker
import androidx.work.ForegroundInfo
import androidx.work.WorkerParameters
import app.smartexplorer.android.api.CatchUpResult
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.api.SyncApi.resultAs
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.service.BackgroundController
import app.smartexplorer.android.service.BackgroundService
import app.smartexplorer.android.service.ServiceNotifications
import app.smartexplorer.android.system.HostMonitor
import app.smartexplorer.android.system.Notifications
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Periodic catch-up (spec F17 "Periodisch"): reports the measured host state, starts
 * `bg.catchUp` and waits for the task to end. A run that takes longer than a moment asks for
 * the foreground (dataSync, best effort); when WorkManager stops the worker, only this run's
 * jobs are cancelled (`task.cancel`).
 */
class SyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val mode = AppPrefs.bgMode.value
        if (mode == BackgroundController.MODE_OFF) return Result.success()
        // Persistent mode: the running service keeps the daemon awake; this work is its fallback.
        if (mode == BackgroundController.MODE_PERSISTENT && BackgroundService.isRunning) return Result.success()

        HostMonitor.pushNow(applicationContext)
        val taskId = try {
            SyncApi.catchUp()
        } catch (e: CoreException) {
            Log.w(TAG, "bg.catchUp not started: ${e.kind}: ${e.displayText()}")
            return Result.success()
        }
        return try {
            coroutineScope {
                val promotion = launch {
                    delay(PROMOTE_AFTER_MS)
                    promote(taskId)
                }
                val end = SyncApi.awaitTask(taskId)
                promotion.cancel()
                val skipped = end.resultAs<CatchUpResult>()?.skipped.orEmpty()
                Log.i(TAG, "catch-up ${end.state}: ${end.message.orEmpty()}; ${skipped.size} skipped")
                Result.success()
            }
        } catch (e: CancellationException) {
            // Stopped by WorkManager (constraints lost, work cancelled): end only this run's jobs.
            withContext(NonCancellable) {
                try {
                    SyncApi.cancelTask(taskId)
                } catch (c: CoreException) {
                    Log.w(TAG, "task.cancel $taskId failed: ${c.kind}: ${c.displayText()}")
                }
            }
            throw e
        } catch (e: CoreException) {
            Log.w(TAG, "catch-up $taskId not followed: ${e.kind}: ${e.displayText()}")
            Result.success()
        }
    }

    /** Foreground for longer runs; refused starts (background limits) only log. */
    private suspend fun promote(taskId: String) {
        try {
            setForeground(
                ForegroundInfo(
                    Notifications.ID_CATCH_UP,
                    ServiceNotifications.catchUp(applicationContext, null),
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
                ),
            )
        } catch (e: CancellationException) {
            throw e
        } catch (e: IllegalStateException) {
            Log.w(TAG, "catch-up stays a background worker", e)
            return
        }
        // Follow the running job in the notification until the run ends (this job is cancelled then).
        Core.tasks
            .map { tasks -> tasks.firstOrNull { it.id == taskId }?.message }
            .distinctUntilChanged()
            .collect { text -> Notifications.notify(applicationContext, Notifications.ID_CATCH_UP, ServiceNotifications.catchUp(applicationContext, text)) }
    }

    private companion object {
        const val TAG = "SmartExplorerWorker"
        const val PROMOTE_AFTER_MS = 3_000L
    }
}
