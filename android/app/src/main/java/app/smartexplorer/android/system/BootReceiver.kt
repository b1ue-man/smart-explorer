package app.smartexplorer.android.system

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.smartexplorer.android.service.BackgroundController

/**
 * Device start and app update (spec F17 "Start nach Neustart", A1): restarts only the specialUse
 * background service – for "Dauerbetrieb" or "Share im Hintergrund erreichbar" – with its wake
 * alarm (alarms do not survive a reboot) and re-plans the periodic work. The dataSync task service
 * is never started from here (not allowed from BOOT_COMPLETED, android-apis.md §4.3). Android
 * delivers neither broadcast to an app that was never opened or was force-stopped (stopped
 * state). "Beim Start"-jobs run once per boot inside the daemon (boot marker from the init
 * configuration).
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED -> BackgroundController.onBoot(context)
            android.app.AlarmManager.ACTION_SCHEDULE_EXACT_ALARM_PERMISSION_STATE_CHANGED -> BackgroundController.refreshSchedule(context)
        }
    }
}
