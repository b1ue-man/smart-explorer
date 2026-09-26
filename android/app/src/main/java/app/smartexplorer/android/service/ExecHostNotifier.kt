package app.smartexplorer.android.service

import android.app.Notification
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.core.app.NotificationCompat
import app.smartexplorer.android.R
import app.smartexplorer.android.api.EXEC_INCOMING
import app.smartexplorer.android.api.ExecJob
import app.smartexplorer.android.api.ExecJobs
import app.smartexplorer.android.api.ShareExecApi
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.system.Notifications
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.MainTab
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.Format
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * Ongoing notification "<Gerät> führt einen Befehl aus" with [Stopp] for every command another
 * device runs on this phone (spec G4 B). Fed by the core's `share` events: a command starting or
 * ending here wakes the Share poller at once, which then sends one, so the notification follows
 * within one `share.execJobs` round trip (the poller's own cadence – up to 60 s in the background –
 * applies only if that wake-up is missed). While a notification is shown it is also re-checked
 * every [RECHECK_MS]. Runs for the whole process once [start]ed.
 */
object ExecHostNotifier {
    private const val TAG = "SmartExplorerExec"
    private const val RECHECK_MS = 5_000L
    private const val CODE_OPEN_SHARE = 2005
    internal const val ACTION_STOP = "app.smartexplorer.android.action.STOP_EXEC"
    internal const val EXTRA_EXEC_ID = "execId"
    internal const val EXTRA_PEER = "peerDeviceId"

    private val started = AtomicBoolean(false)

    // Main thread only: every coroutine of this scope touches [shown] there.
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val refreshes = MutableStateFlow(0L)

    /** Notification id per running command (`execId`). */
    private val shown = LinkedHashMap<String, Int>()

    fun start(context: Context) {
        if (!started.compareAndSet(false, true)) return
        val app = context.applicationContext
        scope.launch { Core.events.collect { if (it is CoreEvent.Share) requestRefresh() } }
        // A StateFlow is conflated: a burst of events leads to at most one more load.
        scope.launch { refreshes.collect { refresh(app) } }
        scope.launch {
            while (true) {
                delay(RECHECK_MS)
                if (shown.isNotEmpty()) requestRefresh()
            }
        }
    }

    private fun requestRefresh() {
        refreshes.update { it + 1 }
    }

    /** [Stopp] of a notification ([ExecHostStopReceiver]); [done] ends the broadcast. */
    internal fun stop(execId: String, peerDeviceId: String, done: () -> Unit) {
        scope.launch {
            try {
                ShareExecApi.cancel(ExecJob(direction = EXEC_INCOMING, execId = execId, peerDeviceId = peerDeviceId))
            } catch (e: CoreException) {
                // `not_found`: it ended meanwhile; the refresh removes its notification.
                Log.w(TAG, "share.cancelExecJob failed: ${e.kind}: ${e.message}")
            } finally {
                requestRefresh()
                done()
            }
        }
    }

    private suspend fun refresh(context: Context) {
        val jobs = try {
            ShareExecApi.jobs()
        } catch (e: CoreException) {
            Log.w(TAG, "share.execJobs failed: ${e.kind}: ${e.message}")
            return
        }
        val running = runningOnThisPhone(jobs).associateBy { it.execId }
        shown.keys.filter { it !in running }.forEach { execId ->
            shown.remove(execId)?.let { Notifications.cancel(context, it) }
        }
        running.values.forEach { job ->
            val id = shown[job.execId] ?: freeExecNotificationId(shown.values)?.also { shown[job.execId] = it }
            if (id != null) Notifications.notify(context, id, notification(context, job, id))
        }
    }

    private fun notification(context: Context, job: ExecJob, id: Int): Notification {
        val stop = PendingIntent.getBroadcast(
            context,
            id,
            Intent(context, ExecHostStopReceiver::class.java)
                .setAction(ACTION_STOP)
                .putExtra(EXTRA_EXEC_ID, job.execId)
                .putExtra(EXTRA_PEER, job.peerDeviceId),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val open = PendingIntent.getActivity(
            context,
            CODE_OPEN_SHARE,
            AppNav.intentFor(context, NavRequest.SelectTab(MainTab.Share)),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val startedText = job.startedAt?.let { "Gestartet ${Format.time(it * 1000)}" }
        return NotificationCompat.Builder(context, Notifications.CHANNEL_EXEC)
            .setSmallIcon(R.drawable.ic_terminal)
            .setContentTitle(execNotificationTitle(job))
            .setContentText(listOfNotNull(execProgramText(job), startedText).joinToString(" · "))
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_STATUS)
            .setContentIntent(open)
            .addAction(R.drawable.ic_close, "Stopp", stop)
            .build()
    }
}

/** [Stopp] in the notification of a running command (not exported). */
class ExecHostStopReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ExecHostNotifier.ACTION_STOP) return
        val execId = intent.getStringExtra(ExecHostNotifier.EXTRA_EXEC_ID) ?: return
        val peer = intent.getStringExtra(ExecHostNotifier.EXTRA_PEER) ?: return
        val pending = goAsync()
        ExecHostNotifier.stop(execId, peer) { pending.finish() }
    }
}

/** Commands other devices run on this phone right now. */
internal fun runningOnThisPhone(jobs: ExecJobs): List<ExecJob> = jobs.active.filter { it.direction == EXEC_INCOMING }

/** "Laptop führt einen Befehl aus". */
internal fun execNotificationTitle(job: ExecJob): String = "${job.peerName.ifBlank { "Ein Gerät" }} führt einen Befehl aus"

/** "Shell-Befehl" or the program of an argument list. */
internal fun execProgramText(job: ExecJob): String =
    if (job.program == "<shell>" || job.program.isBlank()) "Shell-Befehl" else job.program

/** The lowest notification id of the exec range not in [used]; `null` when all are taken. */
internal fun freeExecNotificationId(used: Collection<Int>): Int? =
    (Notifications.ID_EXEC_FIRST until Notifications.ID_EXEC_FIRST + Notifications.EXEC_ID_COUNT).firstOrNull { it !in used }
