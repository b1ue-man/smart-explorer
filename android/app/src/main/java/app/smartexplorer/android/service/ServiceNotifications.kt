package app.smartexplorer.android.service

import android.app.Notification
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import app.smartexplorer.android.R
import app.smartexplorer.android.api.BgStatus
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.system.Notifications
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.MainTab
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.Format

/** Notifications of the task service, the background service and the catch-up worker. */
internal object ServiceNotifications {
    private const val CODE_OPEN_TRANSFERS = 2001
    private const val CODE_OPEN_SYNC = 2002
    private const val CODE_OPEN_BACKGROUND = 2003
    private const val CODE_OPEN_SHARE = 2004
    private const val CODE_CANCEL_TASKS = 2101
    private const val CODE_PAUSE = 2102
    private const val CODE_RESUME = 2103

    /** Kinds shown as "synchronisiert" instead of "überträgt". */
    private val SYNC_KINDS = setOf("sync", "mirror")

    /** "Smart Explorer überträgt – 45 %" with a bar and [Abbrechen] (spec F8). */
    fun tasks(context: Context, tasks: List<TaskInfo>, activeJob: String?): Notification {
        val active = tasks.filter { it.isActive && it.kind != SyncApi.KIND_CATCH_UP }
        val syncOnly = active.all { it.kind in SYNC_KINDS }
        val total = active.sumOf { it.totalBytes }
        val done = active.sumOf { it.doneBytes }
        val percent = Format.percent(done, total)
        val title = buildString {
            append(if (syncOnly) "Smart Explorer synchronisiert" else "Smart Explorer überträgt")
            if (percent != null) append(" – $percent %")
        }
        val text = when {
            active.size == 1 -> active[0].let { task -> task.message?.takeIf { it.isNotBlank() }?.let { "${task.title} · $it" } ?: task.title }
            active.size > 1 -> buildString {
                append("${active.size} Vorgänge")
                if (total > 0) append(" · ${Format.size(done)} von ${Format.size(total)}")
            }
            activeJob != null -> "Sync-Job läuft: $activeJob"
            else -> "Wird beendet…"
        }
        val target = if (syncOnly) NavRequest.SelectTab(MainTab.Sync) else NavRequest.ShowTransfers
        val builder = NotificationCompat.Builder(context, Notifications.CHANNEL_TRANSFERS)
            .setSmallIcon(if (syncOnly) R.drawable.ic_sync else R.drawable.ic_upload)
            .setContentTitle(title)
            .setContentText(text)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .setProgress(100, percent ?: 0, percent == null)
            .setContentIntent(open(context, target, if (syncOnly) CODE_OPEN_SYNC else CODE_OPEN_TRANSFERS))
        if (active.isNotEmpty()) {
            builder.addAction(
                R.drawable.ic_close,
                "Abbrechen",
                serviceAction(context, TaskForegroundService::class.java, TaskForegroundService.ACTION_CANCEL, CODE_CANCEL_TASKS),
            )
        }
        return builder.build()
    }

    /** After `onTimeout`: the system ended the running tasks (spec F8). */
    fun stoppedBySystem(context: Context): Notification =
        NotificationCompat.Builder(context, Notifications.CHANNEL_TRANSFERS)
            .setSmallIcon(R.drawable.ic_warning)
            .setContentTitle("Vom System beendet")
            .setContentText("Android hat laufende Vorgänge nach langer Zeit im Hintergrund abgebrochen.")
            .setAutoCancel(true)
            .setContentIntent(open(context, NavRequest.ShowTransfers, CODE_OPEN_TRANSFERS))
            .build()

    /** "Smart Explorer im Hintergrund – n Jobs, Share online" with [Pausieren]/[Fortsetzen] (spec F17). */
    fun background(context: Context, status: BgStatus?, enabledJobs: Int, shareOnline: Boolean): Notification {
        val paused = status?.paused == true
        val text = when {
            status == null -> "Wird gestartet…"
            else -> buildString {
                append(if (paused) BackgroundText.paused(status) else if (enabledJobs == 1) "1 Job" else "$enabledJobs Jobs")
                append(if (shareOnline) ", Share online" else ", Share offline")
                status.activeJob?.let { append(" · läuft: $it") }
            }
        }
        val builder = NotificationCompat.Builder(context, Notifications.CHANNEL_BACKGROUND)
            .setSmallIcon(R.drawable.ic_sync)
            .setContentTitle("Smart Explorer im Hintergrund")
            .setContentText(text)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(open(context, NavRequest.ShowBackgroundSettings, CODE_OPEN_BACKGROUND))
        if (paused) {
            builder.addAction(
                R.drawable.ic_play,
                "Fortsetzen",
                serviceAction(context, BackgroundService::class.java, BackgroundService.ACTION_RESUME, CODE_RESUME),
            )
        } else {
            builder.addAction(
                R.drawable.ic_pause,
                "Pausieren",
                serviceAction(context, BackgroundService::class.java, BackgroundService.ACTION_PAUSE, CODE_PAUSE),
            )
        }
        return builder.build()
    }

    /** New incoming share request(s); opens the requests on the Share page. */
    fun shareRequest(context: Context, count: Int): Notification =
        NotificationCompat.Builder(context, Notifications.CHANNEL_SHARE)
            .setSmallIcon(R.drawable.ic_share)
            .setContentTitle(if (count == 1) "Neue Share-Anfrage" else "$count neue Share-Anfragen")
            .setContentText("Antippen, um anzunehmen oder abzulehnen.")
            .setAutoCancel(true)
            .setContentIntent(open(context, NavRequest.ShowShareRequests, CODE_OPEN_SHARE))
            .build()

    /** Foreground notification of the periodic catch-up worker. */
    fun catchUp(context: Context, text: String?): Notification =
        NotificationCompat.Builder(context, Notifications.CHANNEL_BACKGROUND)
            .setSmallIcon(R.drawable.ic_sync)
            .setContentTitle("Hintergrund-Sync")
            .setContentText(text?.takeIf { it.isNotBlank() } ?: "Fällige Jobs werden nachgeholt…")
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setProgress(0, 0, true)
            .setContentIntent(open(context, NavRequest.SelectTab(MainTab.Sync), CODE_OPEN_SYNC))
            .build()

    private fun open(context: Context, request: NavRequest, code: Int): PendingIntent =
        PendingIntent.getActivity(
            context,
            code,
            AppNav.intentFor(context, request),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

    private fun serviceAction(context: Context, service: Class<out Service>, action: String, code: Int): PendingIntent =
        PendingIntent.getService(
            context,
            code,
            Intent(context, service).setAction(action),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
}
