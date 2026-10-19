package app.smartexplorer.android.screen

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import app.smartexplorer.android.R
import app.smartexplorer.android.api.ScreenHostSession
import app.smartexplorer.android.core.ScreenHostStart
import app.smartexplorer.android.system.Notifications

/**
 * The host user's view of screen sessions (C7, SCR-M8): the ongoing notification „Bildschirm wird
 * geteilt“ with „Beenden“ for every running session (while MediaProjection runs it is the
 * foreground notification of [ScreenHostService], next to the system's status bar chip), and the
 * question „Freigeben“ for a session that needs the system dialog. „Freigeben“ opens
 * [ScreenConsentActivity] (an activity intent, never a silent action); „Ablehnen“ ends the session.
 */
internal object ScreenHostNotifier {
    const val CHANNEL_SCREEN = "screen_session"
    const val ID_SESSIONS = 1400
    private const val ID_CONSENT_FIRST = 1401
    private const val CONSENT_ID_COUNT = 16

    fun consentId(session: Long): Int = ID_CONSENT_FIRST + (session % CONSENT_ID_COUNT).toInt()

    private fun ensureChannel(context: Context) {
        Notifications.ensureChannels(context)
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        if (manager.getNotificationChannel(CHANNEL_SCREEN) == null) {
            val channel = NotificationChannel(CHANNEL_SCREEN, "Bildschirmfreigabe", NotificationManager.IMPORTANCE_DEFAULT)
            channel.description = "Zeigt, solange ein anderes Gerät diesen Bildschirm sieht oder steuert"
            channel.setSound(null, null)
            manager.createNotificationChannel(channel)
        }
    }

    private fun serviceIntent(context: Context, action: String, session: Long?, code: Int): PendingIntent =
        PendingIntent.getService(
            context,
            code,
            Intent(context, ScreenHostService::class.java).setAction(action).apply { session?.let { putExtra(ScreenHostService.EXTRA_SESSION, it) } },
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

    /** „Bildschirm wird geteilt“ for [sessions], with „Beenden“ for all of them. */
    fun sessionsNotification(context: Context, sessions: List<ScreenHostSession>): Notification {
        ensureChannel(context)
        val names = sessions.map { it.viewerName.ifBlank { "Ein Gerät" } }.distinct()
        val controlled = sessions.any { it.projection }
        return NotificationCompat.Builder(context, CHANNEL_SCREEN)
            .setSmallIcon(R.drawable.ic_lock)
            .setContentTitle(if (controlled) "Bildschirm wird geteilt" else "Bildschirm-Vorschau wird geteilt")
            .setContentText(names.joinToString(", ").ifBlank { "Bildschirmfreigabe läuft" })
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_STATUS)
            .addAction(R.drawable.ic_close, "Beenden", serviceIntent(context, ScreenHostService.ACTION_STOP, null, ID_SESSIONS))
            .build()
    }

    /** The question of [start]: „Freigeben“ opens the consent activity, „Ablehnen“ ends it. */
    fun consentNotification(context: Context, start: ScreenHostStart): Notification {
        ensureChannel(context)
        val id = consentId(start.session)
        val open = ScreenConsentActivity.pendingIntent(context, id, ScreenConsentActivity.sessionIntent(context, start))
        val who = start.viewerName.ifBlank { "Ein Gerät" }
        return NotificationCompat.Builder(context, Notifications.CHANNEL_ACCESS_APPROVAL)
            .setSmallIcon(R.drawable.ic_lock)
            .setContentTitle("$who möchte den Bildschirm sehen")
            .setContentText("Freigeben öffnet die Android-Abfrage „Bildschirm teilen“")
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setContentIntent(open)
            .setDeleteIntent(serviceIntent(context, ScreenHostService.ACTION_DENY, start.session, id))
            .addAction(
                NotificationCompat.Action.Builder(R.drawable.ic_check, "Freigeben", open).setAuthenticationRequired(true).build(),
            )
            .addAction(R.drawable.ic_close, "Ablehnen", serviceIntent(context, ScreenHostService.ACTION_DENY, start.session, id))
            .build()
    }

    fun showConsent(context: Context, start: ScreenHostStart): Boolean =
        Notifications.notify(context, consentId(start.session), consentNotification(context, start))

    fun cancelConsent(context: Context, session: Long) = Notifications.cancel(context, consentId(session))

    /** Shows or removes the ongoing notification of the sessions without MediaProjection. */
    fun showSessions(context: Context, sessions: List<ScreenHostSession>) {
        if (sessions.isEmpty()) {
            Notifications.cancel(context, ID_SESSIONS)
        } else {
            Notifications.notify(context, ID_SESSIONS, sessionsNotification(context, sessions))
        }
    }
}
