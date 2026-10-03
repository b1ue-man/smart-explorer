package app.smartexplorer.android.system

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.SystemClock
import android.util.Log
import app.smartexplorer.android.service.BackgroundController
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * Wake alarm of the background service (spec A2): an inexact `setAndAllowWhileIdle` alarm on
 * `ELAPSED_REALTIME_WAKEUP` every [INTERVAL_MS] starts [KeepAliveReceiver] – in a process
 * Android ended as well – which dispatches a worker and probes the Share connection. A service
 * restart additionally requires the app's battery exemption; this inexact alarm grants none.
 * 10 min = 6 per hour stays within the 7 while-idle alarms per hour an app without battery
 * exemption gets (android-background-reachability.md §1.5) and needs no exact-alarm permission.
 * The alarm is chained: every delivery plans the next one; boot, process start and every
 * service start plan it anew (replacing the pending one). It does not survive a force stop.
 */
object KeepAliveAlarm {
    const val ACTION = "app.smartexplorer.android.action.KEEP_ALIVE"

    private const val TAG = "SmartExplorerKeepAlive"
    private const val INTERVAL_MS = 10 * 60_000L
    private const val REQUEST_CODE = 3001

    /** Wake lock of one delivery: the probe's wait plus restarting the service. */
    private const val DELIVERY_HOLD_MS = KeepAliveProbe.WAIT_MS + 5_000L

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    /** Plans the next alarm [INTERVAL_MS] from now (replaces a pending one). */
    fun schedule(context: Context) {
        val alarms = context.getSystemService(AlarmManager::class.java) ?: return
        try {
            alarms.setAndAllowWhileIdle(
                AlarmManager.ELAPSED_REALTIME_WAKEUP,
                SystemClock.elapsedRealtime() + INTERVAL_MS,
                pendingIntent(context),
            )
        } catch (e: RuntimeException) {
            // SecurityException/IllegalStateException when the per-app alarm limit is reached.
            Log.w(TAG, "keep-alive alarm not planned", e)
        }
    }

    fun cancel(context: Context) {
        context.getSystemService(AlarmManager::class.java)?.cancel(pendingIntent(context))
    }

    /** One delivery: the broadcast stays open ([done] ends it) and the CPU awake until the probe ended. */
    internal fun onAlarm(context: Context, done: () -> Unit) {
        scope.launch {
            try {
                WakeKeeper.whileHeld(DELIVERY_HOLD_MS) {
                    if (BackgroundController.onKeepAlive(context)) {
                        KeepAliveProbe.run(networkChanged = false)
                    }
                }
            } finally {
                done()
            }
        }
    }

    private fun pendingIntent(context: Context): PendingIntent =
        PendingIntent.getBroadcast(
            context,
            REQUEST_CODE,
            Intent(context, KeepAliveReceiver::class.java).setAction(ACTION),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
}

/**
 * Receiver of [KeepAliveAlarm] (not exported). `goAsync` keeps the broadcast – and with it the
 * alarm's own wake lock – open until the probe ended; alarm broadcasts are background broadcasts
 * (60 s limit), the probe needs at most [KeepAliveProbe.WAIT_MS].
 */
class KeepAliveReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != KeepAliveAlarm.ACTION) return
        val pending = goAsync()
        KeepAliveAlarm.onAlarm(context.applicationContext) { pending.finish() }
    }
}
