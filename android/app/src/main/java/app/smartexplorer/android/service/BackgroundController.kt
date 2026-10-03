package app.smartexplorer.android.service

import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.workDataOf
import android.provider.MediaStore
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkInfo
import androidx.work.WorkManager
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.system.HostMonitor
import app.smartexplorer.android.system.KeepAliveAlarm
import app.smartexplorer.android.system.SyncScheduleAlarm
import app.smartexplorer.android.system.Permissions
import app.smartexplorer.android.work.SyncWorker
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * Background mode (spec F17, `AppPrefs.bgMode`): "off" = no scheduled jobs (daemon sync flag
 * off, no periodic work); "periodic" = WorkManager wakes the app for a catch-up run; "persistent"
 * = [BackgroundService] (specialUse) keeps the process and the embedded daemon awake. Independent
 * of the mode, "Share im Hintergrund erreichbar" (spec A1, `AppPrefs.shareReachable`) runs the same
 * service while Share is set up, so other devices reach the phone without the app being opened;
 * outside "Dauerbetrieb" the daemon then still leaves scheduled jobs to the periodic run (A6,
 * `sys.hostState.deferScheduling`). The service is (re)started on boot, app update, every process
 * start, UI start and by the wake alarm ([KeepAliveAlarm], A2). The daemon itself runs once per
 * process and is never stopped for visibility (api.md §4.7).
 */
object BackgroundController {
    const val MODE_OFF = "off"
    const val MODE_PERIODIC = "periodic"
    const val MODE_PERSISTENT = "persistent"

    /** Unique name of the periodic catch-up work. */
    const val WORK_NAME = "background-catch-up"

    private const val TAG = "SmartExplorerBg"
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val syncFlagLock = Mutex()
    private val watching = AtomicBoolean(false)
    private val statusChanges = MutableStateFlow(0L)
    private val scheduleLock = Mutex()
    private val nextJobRun = MutableStateFlow<Long?>(null)
    private const val SCHEDULE_WORK = "sync-scheduled-window"
    private const val CONTENT_WORK = "sync-media-hints"
    const val CONTENT_HINT = "content-hint"

    /** Ticks when the UI changed the daemon's pause or auto-pause ([BackgroundService] reloads). */
    internal val statusChanged: StateFlow<Long> = statusChanges.asStateFlow()

    /** The background service should run: "Dauerbetrieb", or Share reachable and set up (spec A1). */
    fun serviceWanted(): Boolean =
        AppPrefs.bgMode.value == MODE_PERSISTENT || (AppPrefs.shareReachable.value && AppPrefs.shareRunning.value)

    /** From `SmartExplorerApp.onCreate` (any process start: UI, worker, boot, wake alarm). */
    fun onAppStart(context: Context) {
        val app = context.applicationContext
        HostMonitor.start(app)
        scope.launch {
            try {
                SyncApi.ensureDaemon()
            } catch (e: CoreException) {
                Log.w(TAG, "bg.ensureDaemon failed: ${e.kind}: ${e.displayText()}")
            }
        }
        apply(app)
        if (watching.compareAndSet(false, true)) {
            // The switch and the settled Share state start or stop the service on their own; the
            // first value repeats the start decision of apply (idempotent).
            scope.launch {
                combine(AppPrefs.shareReachable, AppPrefs.shareRunning) { reachable, running -> reachable && running }
                    .distinctUntilChanged()
                    .collect { reconcileService(app) }
            }
        }
    }

    /** From `MainActivity.onStart/onStop`: host state, task service and the background service. */
    fun onUiVisible(context: Context, visible: Boolean) {
        val app = context.applicationContext
        HostMonitor.setForeground(visible)
        TaskKeeper.onUiVisible(app, visible)
        // While the UI is visible a foreground service may always start: repairs a start Android
        // refused in the background (boot, alarm, process restart).
        if (visible) reconcileService(app)
    }

    /** Applies the current settings: periodic work (UPDATE policy), sync flag, background service. */
    fun apply(context: Context) {
        val app = context.applicationContext
        val mode = AppPrefs.bgMode.value
        scheduleWork(app, mode)
        armContentWake(app, replace = true)
        scope.launch {
            // One call at a time, each with the mode current at that moment: the last one to run
            // always sends the newest setting, however overlapping calls are ordered.
            syncFlagLock.withLock {
                try {
                    SyncApi.setSyncEnabled(AppPrefs.bgMode.value != MODE_OFF)
                    refreshSchedule(app)
                } catch (e: CoreException) {
                    Log.w(TAG, "bg.setSyncEnabled failed: ${e.kind}: ${e.displayText()}")
                }
            }
        }
        reconcileService(app)
    }

    /**
     * BOOT_COMPLETED / MY_PACKAGE_REPLACED: only the specialUse background service and the
     * periodic work. A dataSync service must never start from BOOT_COMPLETED (android-apis.md §4.3).
     */
    internal fun onBoot(context: Context) {
        val app = context.applicationContext
        scheduleWork(app, AppPrefs.bgMode.value)
        armContentWake(app)
        refreshSchedule(app)
        reconcileService(app)
    }

    /**
     * Wake alarm delivery ([KeepAliveAlarm]): plans the next alarm and restarts the service when
     * the system ended it. `false` (alarm cancelled) when the service is no longer wanted.
     */
    internal fun onKeepAlive(context: Context): Boolean {
        val app = context.applicationContext
        if (!serviceWanted()) {
            KeepAliveAlarm.cancel(app)
            return false
        }
        KeepAliveAlarm.schedule(app)
        // An inexact alarm gives no FGS-start exemption. A battery-exempt
        // app has its own exemption; otherwise the worker remains available.
        if (Permissions.isIgnoringBatteryOptimizations(app)) startBackgroundService(app)
        enqueueCatchUp(app)
        return true
    }

    /** Every start command of [BackgroundService]: keeps the wake alarm planned. */
    internal fun onServiceStarted(context: Context) {
        KeepAliveAlarm.schedule(context.applicationContext)
        refreshSchedule(context.applicationContext)
    }

    /** The UI paused, resumed or changed the auto-pause: the "Dauerbetrieb" notification follows. */
    fun onBackgroundStatusChanged() {
        statusChanges.update { it + 1 }
        scope.launch { appContextForSchedule?.let { refreshSchedule(it) } }
    }

    /**
     * Android refused the background service from the background (no start exemption without
     * the battery-optimization exception, or the app is "Eingeschränkt"); shown in the settings.
     */
    internal fun noteStartRefused() {
        AppPrefs.setBgStartRefusedMs(System.currentTimeMillis())
    }

    /** Next planned run of the periodic work, `null` when none is enqueued. */
    fun nextRun(context: Context): Flow<Long?> = combine(
        WorkManager.getInstance(context.applicationContext)
            .getWorkInfosForUniqueWorkFlow(WORK_NAME)
            .map { infos ->
                infos.firstOrNull { it.state == WorkInfo.State.ENQUEUED }
                    ?.nextScheduleTimeMillis
                    ?.takeIf { it > 0 && it != Long.MAX_VALUE }
            }, nextJobRun,
    ) { periodic, job -> listOfNotNull(periodic, job).minOrNull() }

    @Volatile private var appContextForSchedule: Context? = null

    fun refreshSchedule(context: Context) {
        val app = context.applicationContext
        appContextForSchedule = app
        scope.launch { scheduleLock.withLock {
            if (AppPrefs.bgMode.value == MODE_OFF) {
                nextJobRun.value = null; SyncScheduleAlarm.schedule(app, null); return@withLock
            }
            try {
                val status = SyncApi.status()
                val host = HostMonitor.measure(app)
                val paused = status.paused && status.pausedUntilMs == null ||
                    status.autopauseBattery && host.powerSave || status.autopauseMetered && host.metered
                val due = if (paused) null else status.nextScheduledRunMs?.let { maxOf(it, status.pausedUntilMs ?: 0) }
                nextJobRun.value = due
                SyncScheduleAlarm.schedule(app, due)
            } catch (e: CoreException) { Log.w(TAG, "Echte Jobzeit nicht verfügbar; periodischer Fallback bleibt", e) }
        } }
    }

    fun enqueueCatchUp(context: Context) {
        if (AppPrefs.bgMode.value == MODE_OFF) return
        val request = OneTimeWorkRequestBuilder<SyncWorker>().setConstraints(constraints().build()).build()
        WorkManager.getInstance(context.applicationContext).enqueueUniqueWork(SCHEDULE_WORK, ExistingWorkPolicy.KEEP, request)
    }

    internal fun onScheduleAlarm(context: Context, exact: Boolean) {
        if (AppPrefs.bgMode.value == MODE_OFF) return
        if (exact && serviceWanted()) startBackgroundService(context)
        enqueueCatchUp(context)
        refreshSchedule(context)
    }

    fun armContentWake(context: Context, afterCurrent: Boolean = false, replace: Boolean = false) {
        val work = WorkManager.getInstance(context.applicationContext)
        if (AppPrefs.bgMode.value == MODE_OFF) { work.cancelUniqueWork(CONTENT_WORK); return }
        val request = OneTimeWorkRequestBuilder<SyncWorker>()
            .setInputData(workDataOf(CONTENT_HINT to true))
            .setConstraints(constraints().addContentUriTrigger(MediaStore.Files.getContentUri(MediaStore.VOLUME_EXTERNAL), true)
                .setTriggerContentUpdateDelay(10, TimeUnit.SECONDS).setTriggerContentMaxDelay(60, TimeUnit.SECONDS).build())
            .build()
        work.enqueueUniqueWork(CONTENT_WORK, when {
            afterCurrent -> ExistingWorkPolicy.APPEND_OR_REPLACE
            replace -> ExistingWorkPolicy.REPLACE
            else -> ExistingWorkPolicy.KEEP
        }, request)
    }

    private fun constraints(): Constraints.Builder = Constraints.Builder()
        .setRequiredNetworkType(if (AppPrefs.bgWifiOnly.value) NetworkType.UNMETERED else NetworkType.NOT_REQUIRED)
        .setRequiresCharging(AppPrefs.bgChargingOnly.value)
        .setRequiresBatteryNotLow(AppPrefs.bgBatteryNotLow.value)

    /**
     * Periodic fallback in both enabled modes, alongside actual deadline and
     * content windows. Native admission joins existing runs without duplicating them.
     */
    private fun scheduleWork(context: Context, mode: String) {
        val workManager = WorkManager.getInstance(context)
        if (mode == MODE_OFF) {
            workManager.cancelUniqueWork(WORK_NAME)
            workManager.cancelUniqueWork(SCHEDULE_WORK)
            SyncScheduleAlarm.schedule(context, null)
            return
        }
        val constraints = constraints().build()
        val request = PeriodicWorkRequestBuilder<SyncWorker>(AppPrefs.bgIntervalMin.value.toLong(), TimeUnit.MINUTES)
            .setConstraints(constraints)
            .build()
        workManager.enqueueUniquePeriodicWork(WORK_NAME, ExistingPeriodicWorkPolicy.UPDATE, request)
    }

    /** Starts or stops the background service and its wake alarm to match [serviceWanted]. */
    private fun reconcileService(context: Context) {
        if (serviceWanted()) {
            KeepAliveAlarm.schedule(context)
            startBackgroundService(context)
        } else {
            KeepAliveAlarm.cancel(context)
            stopBackgroundService(context)
        }
    }

    private fun startBackgroundService(context: Context) {
        if (BackgroundService.isRunning) return
        try {
            ContextCompat.startForegroundService(context, Intent(context, BackgroundService::class.java))
        } catch (e: IllegalStateException) {
            // ForegroundServiceStartNotAllowedException (API 31+) is an IllegalStateException:
            // started from the background without an exemption. The wake alarm, the next UI start
            // or boot retries.
            Log.w(TAG, "background service not started", e)
            noteStartRefused()
        } catch (e: SecurityException) {
            Log.w(TAG, "background service not started", e)
            noteStartRefused()
        }
    }

    private fun stopBackgroundService(context: Context) {
        context.stopService(Intent(context, BackgroundService::class.java))
    }
}
