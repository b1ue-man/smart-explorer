package app.smartexplorer.android.service

import android.app.Notification
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
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * Background service (specialUse) that keeps the process and with it the embedded daemon
 * (scheduler, real-time jobs, Share host) alive. Two roles, decided by the mode:
 * "Dauerbetrieb" shows "Smart Explorer im Hintergrund – n Jobs, Share online" with
 * [Pausieren]/[Fortsetzen] (spec F17); otherwise it runs for "Share im Hintergrund erreichbar"
 * only and shows "Share erreichbar" (spec A1, A7). Both post a notification for incoming share
 * requests while the UI is not visible. Sticky; started by [BackgroundController] (app start, UI,
 * boot, wake alarm). The notification follows events, not a timer (spec A4).
 */
class BackgroundService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val refreshRequests = Channel<Unit>(Channel.CONFLATED)
    private var running = false
    private var status: BgStatus? = null
    private var enabledJobs = 0
    private var rendered: Rendered? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        runningFlow.value = true
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // startForeground first: a start through startForegroundService requires it in any case.
        if (!promote()) {
            stopSelf(startId)
            return START_NOT_STICKY
        }
        if (!BackgroundController.serviceWanted()) {
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            stopSelf(startId)
            return START_NOT_STICKY
        }
        BackgroundController.onServiceStarted(this)
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
        runningFlow.value = false
        scope.cancel()
        super.onDestroy()
    }

    private fun promote(): Boolean {
        val content = content()
        return try {
            ServiceCompat.startForeground(this, Notifications.ID_BACKGROUND, content.build(), ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
            rendered = content.key
            true
        } catch (e: IllegalStateException) {
            // ForegroundServiceStartNotAllowedException (API 31+) is an IllegalStateException.
            Log.w(TAG, "background service could not enter the foreground", e)
            BackgroundController.noteStartRefused()
            false
        } catch (e: SecurityException) {
            Log.w(TAG, "background service could not enter the foreground", e)
            BackgroundController.noteStartRefused()
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
                val wait = followUpMs()
                if (wait == null) refreshRequests.receive() else withTimeoutOrNull(wait) { refreshRequests.receive() }
            }
        }
        scope.launch {
            Core.events.collect { event ->
                when (event) {
                    is CoreEvent.Jobs -> requestRefresh()
                    is CoreEvent.ShareRequest -> notifyShareRequest(event.count)
                    else -> Unit
                }
            }
        }
        scope.launch { HostMonitor.share.collect { render() } }
        // Switching between "Dauerbetrieb" and the Share-only role changes the notification.
        scope.launch { AppPrefs.bgMode.collect { requestRefresh() } }
        // Pause and auto-pause change without a `jobs` event: from the settings and the host state.
        scope.launch { BackgroundController.statusChanged.collect { requestRefresh() } }
        scope.launch { HostMonitor.hostState.collect { if (persistentRole()) requestRefresh() } }
    }

    private fun requestRefresh() {
        refreshRequests.trySend(Unit)
    }

    private suspend fun control(failure: String, action: suspend () -> Unit) {
        try {
            action()
        } catch (e: CoreException) {
            Log.w(TAG, "$failure: ${e.kind}: ${e.displayText()}")
        }
        requestRefresh()
    }

    /** Loads what the notification shows ("Dauerbetrieb" only) and renders it. */
    private suspend fun refresh() {
        if (persistentRole()) {
            try {
                status = SyncApi.status()
                enabledJobs = SyncApi.jobs().count { it.enabled }
            } catch (e: CoreException) {
                Log.w(TAG, "background status failed: ${e.kind}: ${e.displayText()}")
            }
        }
        render()
    }

    /**
     * The only timed re-checks: the end of a timed pause, a running job (its end may come without
     * a `jobs` event) and a status that failed to load. Everything else follows events.
     */
    private fun followUpMs(): Long? {
        if (!persistentRole()) return null
        val current = status ?: return RETRY_MS
        val pausedUntil = current.pausedUntilMs
        if (current.paused && pausedUntil != null) {
            return (pausedUntil - System.currentTimeMillis()).coerceAtLeast(0) + PAUSE_END_MARGIN_MS
        }
        return if (current.activeJob != null) ACTIVE_JOB_RECHECK_MS else null
    }

    private fun render() {
        val content = content()
        if (content.key == rendered) return
        if (Notifications.notify(this, Notifications.ID_BACKGROUND, content.build())) rendered = content.key
    }

    private fun persistentRole(): Boolean = AppPrefs.bgMode.value == BackgroundController.MODE_PERSISTENT

    private fun content(): Content {
        if (persistentRole()) {
            val text = BackgroundText.persistentText(status, enabledJobs, HostMonitor.share.value.online)
            val paused = status?.paused == true
            return Content(Rendered(persistent = true, text = text, paused = paused)) {
                ServiceNotifications.background(this, text, paused)
            }
        }
        val text = BackgroundText.reachableText(HostMonitor.share.value)
        return Content(Rendered(persistent = false, text = text, paused = false)) {
            ServiceNotifications.shareReachable(this, text)
        }
    }

    /** While the UI is visible the tab badge shows new requests instead. */
    private fun notifyShareRequest(count: Int) {
        if (TaskKeeper.uiVisible) return
        Notifications.notify(this, Notifications.ID_SHARE_REQUEST, ServiceNotifications.shareRequest(this, count))
    }

    /** What the shown notification depends on; an equal key skips the update. */
    private data class Rendered(val persistent: Boolean, val text: String, val paused: Boolean)

    private class Content(val key: Rendered, val build: () -> Notification)

    companion object {
        const val ACTION_PAUSE = "app.smartexplorer.android.action.PAUSE_BACKGROUND"
        const val ACTION_RESUME = "app.smartexplorer.android.action.RESUME_BACKGROUND"

        private val runningFlow = MutableStateFlow(false)

        /** `true` while an instance exists in this process (settings status line). */
        val running: StateFlow<Boolean> = runningFlow.asStateFlow()

        /** [running] now (read by the worker and the controller). */
        val isRunning: Boolean
            get() = runningFlow.value

        private const val TAG = "SmartExplorerBg"
        private const val PAUSE_SECONDS = 3_600L
        private const val RETRY_MS = 30_000L
        private const val ACTIVE_JOB_RECHECK_MS = 30_000L
        private const val PAUSE_END_MARGIN_MS = 1_000L
    }
}
