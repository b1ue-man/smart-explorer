package app.smartexplorer.android.service

import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import android.util.Log
import androidx.core.app.ServiceCompat
import app.smartexplorer.android.api.BgStatus
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.system.HostMonitor
import app.smartexplorer.android.system.Notifications
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Persistent background mode (spec F17, specialUse): keeps the process and with it the embedded
 * daemon (scheduler, real-time jobs, Share host) awake. Shows "Smart Explorer im Hintergrund –
 * n Jobs, Share online" with [Pausieren]/[Fortsetzen] and posts a notification for incoming share
 * requests while the UI is not visible. Started by [BackgroundController] (app start, UI, boot).
 */
class BackgroundService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var running = false
    private var status: BgStatus? = null
    private var enabledJobs = 0

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        isRunning = true
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // startForeground first: a start through startForegroundService requires it in any case.
        if (!promote()) {
            stopSelf(startId)
            return START_NOT_STICKY
        }
        if (AppPrefs.bgMode.value != BackgroundController.MODE_PERSISTENT) {
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            stopSelf(startId)
            return START_NOT_STICKY
        }
        when (intent?.action) {
            ACTION_PAUSE -> scope.launch { control("Nicht pausiert") { SyncApi.pause(PAUSE_SECONDS) } }
            ACTION_RESUME -> scope.launch { control("Nicht fortgesetzt") { SyncApi.resume() } }
        }
        if (!running) {
            running = true
            run()
        }
        return START_STICKY
    }

    override fun onDestroy() {
        isRunning = false
        scope.cancel()
        super.onDestroy()
    }

    private fun promote(): Boolean {
        val notification = ServiceNotifications.background(this, status, enabledJobs, HostMonitor.shareOnline.value)
        return try {
            ServiceCompat.startForeground(this, Notifications.ID_BACKGROUND, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
            true
        } catch (e: IllegalStateException) {
            // ForegroundServiceStartNotAllowedException (API 31+) is an IllegalStateException.
            Log.w(TAG, "background service could not enter the foreground", e)
            false
        } catch (e: SecurityException) {
            Log.w(TAG, "background service could not enter the foreground", e)
            false
        }
    }

    private fun run() {
        scope.launch {
            try {
                SyncApi.ensureDaemon()
            } catch (e: CoreException) {
                Log.w(TAG, "bg.ensureDaemon failed: ${e.kind}: ${e.displayText()}")
            }
            while (true) {
                refresh()
                delay(REFRESH_INTERVAL_MS)
            }
        }
        scope.launch {
            Core.events.collect { event ->
                when (event) {
                    is CoreEvent.Jobs -> refresh()
                    is CoreEvent.ShareRequest -> notifyShareRequest(event.count)
                    else -> Unit
                }
            }
        }
        scope.launch { HostMonitor.shareOnline.collect { render() } }
    }

    private suspend fun control(failure: String, action: suspend () -> Unit) {
        try {
            action()
        } catch (e: CoreException) {
            Log.w(TAG, "$failure: ${e.kind}: ${e.displayText()}")
        }
        refresh()
    }

    private suspend fun refresh() {
        try {
            status = SyncApi.status()
            enabledJobs = SyncApi.jobs().count { it.enabled }
        } catch (e: CoreException) {
            Log.w(TAG, "background status failed: ${e.kind}: ${e.displayText()}")
        }
        render()
    }

    private fun render() {
        Notifications.notify(
            this,
            Notifications.ID_BACKGROUND,
            ServiceNotifications.background(this, status, enabledJobs, HostMonitor.shareOnline.value),
        )
    }

    /** While the UI is visible the tab badge shows new requests instead. */
    private fun notifyShareRequest(count: Int) {
        if (TaskKeeper.uiVisible) return
        Notifications.notify(this, Notifications.ID_SHARE_REQUEST, ServiceNotifications.shareRequest(this, count))
    }

    companion object {
        const val ACTION_PAUSE = "app.smartexplorer.android.action.PAUSE_BACKGROUND"
        const val ACTION_RESUME = "app.smartexplorer.android.action.RESUME_BACKGROUND"

        /** `true` while an instance exists in this process (read by the worker and the controller). */
        @Volatile
        var isRunning = false
            private set

        private const val TAG = "SmartExplorerBg"
        private const val PAUSE_SECONDS = 3_600L
        private const val REFRESH_INTERVAL_MS = 60_000L
    }
}
