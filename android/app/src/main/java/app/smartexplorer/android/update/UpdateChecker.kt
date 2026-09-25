package app.smartexplorer.android.update

import android.app.PendingIntent
import android.content.Context
import android.util.Log
import androidx.core.app.NotificationCompat
import app.smartexplorer.android.R
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.UpdateApi
import app.smartexplorer.android.api.UpdateDownload
import app.smartexplorer.android.api.UpdateInfo
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.system.Notifications
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.Snackbars
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** What the update card shows (spec F22). */
sealed interface UpdateState {
    /** Nothing checked in this process yet. */
    data object Idle : UpdateState

    data object Checking : UpdateState

    data class UpToDate(val current: String) : UpdateState

    data class Available(val info: UpdateInfo) : UpdateState

    /** [taskId] is `null` until the core accepted the download. */
    data class Downloading(val info: UpdateInfo, val taskId: String?) : UpdateState

    /** Verified APK in `<cache>/update/`; [needsPermission] = "Unbekannte Apps installieren" missing. */
    data class Ready(val info: UpdateInfo, val path: String, val needsPermission: Boolean) : UpdateState

    /** [info] set: the download failed (retry downloads); otherwise the check failed. */
    data class Failed(val message: String, val info: UpdateInfo?) : UpdateState
}

/**
 * Update checks against the feed (`update.check`), the APK download (`update.download`, SHA-256
 * checked by the core) and the hand-over to the system installer. The automatic check runs at UI
 * start at most once per day and only while `AppPrefs.autoUpdateCheck` is on.
 */
object UpdateChecker {
    private const val TAG = "SmartExplorerUpdate"
    private const val CHECK_INTERVAL_MS = 24L * 60 * 60 * 1000
    private const val CODE_OPEN_UPDATE = 2201

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val stateFlow = MutableStateFlow<UpdateState>(UpdateState.Idle)
    private val busy = AtomicBoolean(false)

    val state: StateFlow<UpdateState> = stateFlow.asStateFlow()

    /** Automatic check on UI start: returns at once; at most one check per day. */
    fun maybeCheck(context: Context) {
        if (!AppPrefs.autoUpdateCheck.value) return
        val now = System.currentTimeMillis()
        val last = AppPrefs.lastUpdateCheckMs.value
        // A clock set back (now < last) counts as due.
        if (now >= last && now - last < CHECK_INTERVAL_MS) return
        val app = context.applicationContext
        scope.launch { check(app, automatic = true) }
    }

    /** [Jetzt prüfen]; ignored while a check or download runs or an APK is ready. */
    fun checkNow(context: Context) {
        val app = context.applicationContext
        scope.launch { check(app, automatic = false) }
    }

    /** [Installieren] on an available version: download, then open the system installer. */
    fun download(context: Context) {
        val info = when (val current = stateFlow.value) {
            is UpdateState.Available -> current.info
            is UpdateState.Failed -> current.info ?: return
            else -> return
        }
        if (!busy.compareAndSet(false, true)) return
        val app = context.applicationContext
        Notifications.cancel(app, Notifications.ID_UPDATE)
        stateFlow.value = UpdateState.Downloading(info, null)
        scope.launch {
            try {
                stateFlow.value = downloadVerified(info)
            } finally {
                busy.set(false)
            }
            // Straight to the system installer; activity starts belong on the main thread.
            if (stateFlow.value is UpdateState.Ready) withContext(Dispatchers.Main) { install(app) }
        }
    }

    fun cancelDownload() {
        val taskId = (stateFlow.value as? UpdateState.Downloading)?.taskId ?: return
        scope.launch {
            try {
                FilesApi.cancelTask(taskId)
            } catch (e: CoreException) {
                Log.w(TAG, "update download not canceled: ${e.kind}: ${e.message}")
            }
        }
    }

    /** Hands the ready APK to the system installer; the result also updates [state]. */
    internal fun install(context: Context): UpdateInstaller.Outcome? {
        val ready = stateFlow.value as? UpdateState.Ready ?: return null
        val outcome = UpdateInstaller.install(context, ready.path)
        stateFlow.value = when (outcome) {
            UpdateInstaller.Outcome.Started -> ready.copy(needsPermission = false)
            UpdateInstaller.Outcome.NeedsPermission -> ready.copy(needsPermission = true)
            UpdateInstaller.Outcome.NoInstaller -> ready
            UpdateInstaller.Outcome.InvalidFile ->
                UpdateState.Failed("Update-Datei nicht gefunden – bitte erneut laden.", ready.info)
        }
        return outcome
    }

    private suspend fun check(context: Context, automatic: Boolean) {
        val previous = stateFlow.value
        if (previous is UpdateState.Downloading || previous is UpdateState.Ready) return
        if (!busy.compareAndSet(false, true)) return
        try {
            if (!automatic) stateFlow.value = UpdateState.Checking
            val info = UpdateApi.check()
            AppPrefs.setLastUpdateCheckMs(System.currentTimeMillis())
            stateFlow.value = if (info.available) UpdateState.Available(info) else UpdateState.UpToDate(info.current)
            if (info.available && automatic) announce(context, info)
        } catch (e: CoreException) {
            if (automatic) {
                // Offline at start is normal: stay quiet and try again at the next start.
                Log.w(TAG, "automatic update check failed: ${e.kind}: ${e.message}")
                stateFlow.value = previous
            } else {
                stateFlow.value = UpdateState.Failed("Prüfung fehlgeschlagen: ${e.message ?: e.kind}", null)
            }
        } finally {
            busy.set(false)
        }
    }

    private suspend fun downloadVerified(info: UpdateInfo): UpdateState = try {
        val taskId = UpdateApi.download()
        stateFlow.value = UpdateState.Downloading(info, taskId)
        val task = FilesApi.awaitTask(taskId)
        when (task.state) {
            "done" -> {
                val result = FilesApi.resultOf(task, UpdateDownload.serializer())
                    ?: throw CoreException("internal", "Download ohne Ergebnis")
                UpdateState.Ready(info.copy(latest = result.version.ifBlank { info.latest }), result.path, needsPermission = false)
            }
            "canceled" -> UpdateState.Available(info)
            // A wrong checksum ends the task with the core's message ("Download beschädigt – nicht installiert").
            else -> UpdateState.Failed(task.message?.takeIf { it.isNotBlank() } ?: "Download fehlgeschlagen", info)
        }
    } catch (e: CoreException) {
        UpdateState.Failed("Download fehlgeschlagen: ${e.message ?: e.kind}", info)
    }

    /** Notification (channel "updates") plus an in-app hint; both lead to the update card. */
    private fun announce(context: Context, info: UpdateInfo) {
        val open = PendingIntent.getActivity(
            context,
            CODE_OPEN_UPDATE,
            AppNav.intentFor(context, NavRequest.ShowUpdate),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(context, Notifications.CHANNEL_UPDATES)
            .setSmallIcon(R.drawable.ic_update)
            .setContentTitle("Version ${info.latest} verfügbar")
            .setContentText("Antippen, um Smart Explorer zu aktualisieren")
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        Notifications.notify(context, Notifications.ID_UPDATE, notification)
        Snackbars.show("Version ${info.latest} verfügbar", "Anzeigen") { AppNav.send(NavRequest.ShowUpdate) }
    }
}
