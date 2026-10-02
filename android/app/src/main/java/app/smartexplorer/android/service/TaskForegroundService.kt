package app.smartexplorer.android.service

import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import android.os.PowerManager
import android.os.SystemClock
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
import kotlinx.coroutines.launch

/**
 * dataSync foreground service for transfers, "Jetzt" sync runs and the Google sign-in while the
 * UI is not visible (spec F8). Started only by [TaskKeeper]; checks every 5 s and stops itself
 * when neither user tasks nor a daemon job run. The notification shows the progress with
 * [Abbrechen]; on Android 15+ `onTimeout` cancels all tasks and stops (spec F8, "vom System beendet").
 * While a task against another device runs ([TaskKeeper.keepCpuAwake]) it holds a partial wake
 * lock: a foreground service alone does not keep the CPU running with the screen off.
 */
class TaskForegroundService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var watcher: Job? = null
    private var foreground = false
    private var lastStartId = 0

    @Volatile
    private var activeJob: String? = null

    /** Held while a task against another device runs; main thread only. */
    private var cpuLock: PowerManager.WakeLock? = null
    private var cpuAcquiredAt = 0L

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        scope.launch {
            // Task events arrive up to 4/s per task; the notification follows at most once a second.
            // StateFlow collection is conflated by itself (`StateFlow.conflate()` is a deprecation error).
            Core.tasks.collect { tasks ->
                if (foreground) render(tasks)
                holdCpu(tasks)
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
        releaseCpu()
        leaveForeground()
        Notifications.notify(this, Notifications.ID_TRANSFERS, ServiceNotifications.stoppedBySystem(this))
        stopSelf()
    }

    override fun onDestroy() {
        scope.cancel()
        releaseCpu()
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
            holdCpu(Core.tasks.value)
            if (!TaskKeeper.keepsAlive(Core.tasks.value) && activeJob == null) {
                releaseCpu()
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

    /**
     * Holds the CPU while a task against another device runs and lets it go as soon as none does.
     * The lock is taken with a timeout ([CPU_HOLD_MS]) and renewed well before it ends, so a stuck
     * service can never keep the device awake for long.
     */
    private fun holdCpu(tasks: List<TaskInfo>) {
        if (!foreground || !TaskKeeper.needsCpu(tasks)) {
            releaseCpu()
            return
        }
        val lock = cpuLock ?: newCpuLock()?.also { cpuLock = it } ?: return
        val now = SystemClock.elapsedRealtime()
        if (lock.isHeld && now - cpuAcquiredAt < CPU_RENEW_MS) return
        try {
            // Not reference-counted: acquiring again replaces the pending timeout.
            lock.acquire(CPU_HOLD_MS)
            cpuAcquiredAt = now
        } catch (e: RuntimeException) {
            Log.w(TAG, "task wake lock not taken", e)
        }
    }

    private fun releaseCpu() {
        val lock = cpuLock ?: return
        try {
            if (lock.isHeld) lock.release()
        } catch (e: RuntimeException) {
            Log.w(TAG, "task wake lock not released", e)
        }
    }

    private fun newCpuLock(): PowerManager.WakeLock? = getSystemService(PowerManager::class.java)
        ?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, CPU_LOCK_TAG)
        ?.apply { setReferenceCounted(false) }

    companion object {
        /** Notification action [Abbrechen]: cancels every running user task. */
        const val ACTION_CANCEL = "app.smartexplorer.android.action.CANCEL_TASKS"
        private const val TAG = "SmartExplorerTasks"
        private const val CHECK_INTERVAL_MS = 5_000L
        private const val RENDER_INTERVAL_MS = 1_000L

        /** "app:purpose", as `dumpsys power` and batterystats show it. */
        private const val CPU_LOCK_TAG = "SmartExplorer:remote-task"

        /** Upper bound of one hold (the documented example); renewed while the task runs. */
        private const val CPU_HOLD_MS = 10 * 60_000L

        /** Renewal long before the hold ends; the watch checks every [CHECK_INTERVAL_MS]. */
        private const val CPU_RENEW_MS = 60_000L
    }
}
