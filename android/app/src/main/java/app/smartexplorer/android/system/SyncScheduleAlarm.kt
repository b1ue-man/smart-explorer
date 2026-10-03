package app.smartexplorer.android.system

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import app.smartexplorer.android.service.BackgroundController
import kotlinx.coroutines.*

/** One alarm for the earliest actual saved-job deadline. An inexact alarm
 * dispatches WorkManager; only an exact alarm may restart the wanted FGS. */
object SyncScheduleAlarm {
    const val ACTION = "app.smartexplorer.android.action.SYNC_SCHEDULE"
    private const val CODE = 3002
    private const val EXACT = "exact"
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var planned: Pair<Long, Boolean>? = null

    @Synchronized
    fun schedule(context: Context, dueMs: Long?) {
        val alarms = context.getSystemService(AlarmManager::class.java) ?: return
        if (dueMs == null) { alarms.cancel(intent(context, false)); planned = null; return }
        val exact = Permissions.canScheduleExactAlarms(context)
        if (planned == (dueMs to exact)) return
        val at = dueMs.coerceAtLeast(System.currentTimeMillis() + 30_000)
        try {
            if (exact) alarms.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, intent(context, true))
            else alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, intent(context, false))
            planned = dueMs to exact
        } catch (e: SecurityException) {
            try {
                alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, intent(context, false))
                planned = dueMs to false
            } catch (fallback: RuntimeException) { Log.w("SmartExplorerSync", "Jobalarm nicht geplant", fallback) }
        } catch (e: RuntimeException) { Log.w("SmartExplorerSync", "Jobalarm nicht geplant", e) }
    }

    internal fun delivered(context: Context, exact: Boolean, done: () -> Unit) {
        synchronized(this) { planned = null }
        scope.launch {
            try {
                WakeKeeper.whileHeld(15_000) {
                    BackgroundController.onScheduleAlarm(context, exact && Permissions.canScheduleExactAlarms(context))
                }
            } finally { done() }
        }
    }

    private fun intent(context: Context, exact: Boolean): PendingIntent = PendingIntent.getBroadcast(
        context, CODE, Intent(context, SyncScheduleReceiver::class.java).setAction(ACTION).putExtra(EXACT, exact),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
    internal fun isExact(intent: Intent): Boolean = intent.getBooleanExtra(EXACT, false)
}

class SyncScheduleReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != SyncScheduleAlarm.ACTION) return
        val pending = goAsync()
        SyncScheduleAlarm.delivered(context.applicationContext, SyncScheduleAlarm.isExact(intent)) { pending.finish() }
    }
}
