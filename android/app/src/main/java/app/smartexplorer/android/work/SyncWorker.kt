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
import app.smartexplorer.android.service.ServiceNotifications
import app.smartexplorer.android.system.HostMonitor
import app.smartexplorer.android.system.Notifications
import app.smartexplorer.android.system.WakeKeeper
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

/**
 * Periodic catch-up (spec F17 "Periodisch"): reports the measured host state, starts
 * `bg.catchUp` and waits for the task to end. A run that takes longer than a moment asks for
 * the foreground (dataSync, best effort); when WorkManager stops the worker, only this run's
 * jobs are cancelled (`task.cancel`). It also runs while the background service keeps the
 * process alive for "Share im Hintergrund erreichbar". During this bounded worker window the
 * daemon defers independent new scheduling; catch-up owns and cancels only its admitted jobs.
 */
class SyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    private var rearmContent = false
    override suspend fun doWork(): Result {
        val mode = AppPrefs.bgMode.value
        if (mode == BackgroundController.MODE_OFF) return Result.success()
        try {
            // Inside the try: a stop while the host state is sent still closes the gate below.
            HostMonitor.beginWorkerRun(applicationContext)
            var result: Result = Result.retry()
            WakeKeeper.whileHeld(WINDOW_MS + 30_000) {
                result = withTimeoutOrNull(WINDOW_MS) { catchUp() } ?: Result.retry()
            }
            if (rearmContent && inputData.getBoolean(BackgroundController.CONTENT_HINT, false)) {
                BackgroundController.armContentWake(applicationContext, afterCurrent = true)
            }
            return result
        } finally {
            // Also after a stop by WorkManager: the gate closes again.
            withContext(NonCancellable) { HostMonitor.endWorkerRun(applicationContext) }
            BackgroundController.refreshSchedule(applicationContext)
        }
    }

    private suspend fun catchUp(): Result {
        val taskId = try {
            // Not cancellable: a stop during the (possibly seconds long) start must still get the
            // task id, so the catch below can end this run with `task.cancel`.
            withContext(NonCancellable) { SyncApi.catchUp() }
        } catch (e: CoreException) {
            Log.w(TAG, "bg.catchUp not started: ${e.kind}: ${e.displayText()}")
            return if (e.kind in setOf("invalid", "permission", "auth")) Result.success() else Result.retry()
        }
        var ended = false
        return try {
            coroutineScope {
                val promotion = launch {
                    delay(PROMOTE_AFTER_MS)
                    promote(taskId)
                }
                val end = SyncApi.awaitTask(taskId)
                ended = true
                promotion.cancel()
                val result = end.resultAs<CatchUpResult>()
                rearmContent = end.state == "done" && result != null && !result.retrySuggested
                val skipped = result?.skipped.orEmpty()
                Log.i(TAG, "catch-up ${end.state}: ${end.message.orEmpty()}; ${skipped.size} skipped")
                if (end.state != "done" || result == null || result.retrySuggested) Result.retry() else Result.success()
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: CoreException) {
            Log.w(TAG, "catch-up $taskId not followed: ${e.kind}: ${e.displayText()}")
            Result.retry()
        } finally {
            if (!ended) withContext(NonCancellable) {
                runCatching { SyncApi.cancelTask(taskId) }.onFailure { Log.w(TAG, "catch-up cancel failed", it) }
            }
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
        } catch (e: SecurityException) {
            Log.w(TAG, "catch-up foreground permission missing", e)
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
        const val WINDOW_MS = 8 * 60_000L
    }
}
