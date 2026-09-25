package app.smartexplorer.android.service

import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import android.util.Log
import androidx.core.app.ServiceCompat
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.system.Notifications
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.conflate
import kotlinx.coroutines.launch

/**
 * dataSync foreground service for transfers, "Jetzt" sync runs and the Google sign-in while the
 * UI is not visible (spec F8). Started only by [TaskKeeper]; checks every 5 s and stops itself
 * when neither user tasks nor a daemon job run. The notification shows the progress with
 * [Abbrechen]; on Android 15+ `onTimeout` cancels all tasks and stops (spec F8, "vom System beendet").
 */
class TaskForegroundService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var watcher: Job? = null
    private var foreground = false
    private var lastStartId = 0

    @Volatile
    private var activeJob: String? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        scope.launch {
            // Task events arrive up to 4/s per task; the notification follows at most once a second.
            Core.tasks.conflate().collect { tasks ->
                if (foreground) render(tasks)
                delay(RENDER_INTERVAL_MS)
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        lastStartId = startId
        if (intent?.action == ACTION_CANCEL) {
            // Sent by the notification action through startService; no foreground start pending.
            TaskKeeper.cancelUserTasks()
            if (!foreground) stopSelf(startId)
            return START_NOT_STICKY
        }
        if (!promote()) {
            stopSelf(startId)
            return START_NOT_STICKY
        }
        if (watcher == null) watcher = scope.launch { watch() }
        return START_NOT_STICKY
    }

    /** Android 15+: the dataSync time limit in the background was reached. */
    override fun onTimeout(startId: Int, fgsType: Int) {
        Log.w(TAG, "foreground time limit reached, cancelling all tasks")
        TaskKeeper.cancelAllAfterTimeout()
        leaveForeground()
        Notifications.notify(this, Notifications.ID_TRANSFERS, ServiceNotifications.stoppedBySystem(this))
        stopSelf()
    }

    override fun onDestroy() {
        scope.cancel()
        foreground = false
        super.onDestroy()
    }

    /** `startForeground` right away (required after `startForegroundService`); `false` if refused. */
    private fun promote(): Boolean {
        val notification = ServiceNotifications.tasks(this, Core.tasks.value, activeJob)
        return try {
            ServiceCompat.startForeground(this, Notifications.ID_TRANSFERS, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
            foreground = true
            true
        } catch (e: IllegalStateException) {
            // ForegroundServiceStartNotAllowedException (API 31+) is an IllegalStateException.
            Log.w(TAG, "task service could not enter the foreground", e)
            false
        } catch (e: SecurityException) {
            Log.w(TAG, "task service could not enter the foreground", e)
            false
        }
    }

    private fun render(tasks: List<TaskInfo>) {
        Notifications.notify(this, Notifications.ID_TRANSFERS, ServiceNotifications.tasks(this, tasks, activeJob))
    }

    private suspend fun watch() {
        while (true) {
            delay(CHECK_INTERVAL_MS)
            activeJob = try {
                SyncApi.status().activeJob
            } catch (e: CoreException) {
                Log.w(TAG, "bg.status failed: ${e.kind}: ${e.message}")
                null
            }
            if (!TaskKeeper.keepsAlive(Core.tasks.value) && activeJob == null) {
                leaveForeground()
                watcher = null
                // Keeps running when a newer start arrived meanwhile; that start promotes again.
                stopSelf(lastStartId)
                return
            }
        }
    }

    private fun leaveForeground() {
        if (!foreground) return
        foreground = false
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
    }

    companion object {
        /** Notification action [Abbrechen]: cancels every running user task. */
        const val ACTION_CANCEL = "app.smartexplorer.android.action.CANCEL_TASKS"
        private const val TAG = "SmartExplorerTasks"
        private const val CHECK_INTERVAL_MS = 5_000L
        private const val RENDER_INTERVAL_MS = 1_000L
    }
}
