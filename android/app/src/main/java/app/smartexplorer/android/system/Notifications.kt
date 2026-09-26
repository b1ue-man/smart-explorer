package app.smartexplorer.android.system

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.util.Log
import androidx.core.app.NotificationManagerCompat
import app.smartexplorer.android.R

/** Notification channels and a permission-safe notify (android-apis.md §2). */
object Notifications {
    const val CHANNEL_TRANSFERS = "transfers"
    const val CHANNEL_BACKGROUND = "background"
    const val CHANNEL_UPDATES = "updates"
    const val CHANNEL_SHARE = "share"

    /** Ongoing notifications of commands other devices run on this phone (ExecHostNotifier). */
    const val CHANNEL_EXEC = "exec"

    // Notification ids used across the app (one place so services never collide).
    const val ID_TRANSFERS = 1001
    const val ID_BACKGROUND = 1002
    const val ID_CATCH_UP = 1003
    const val ID_UPDATE = 1004
    const val ID_SHARE_REQUEST = 1005

    /** One id per running command of another device, from [ID_EXEC_FIRST] on. */
    const val ID_EXEC_FIRST = 1100
    const val EXEC_ID_COUNT = 50

    private const val TAG = "SmartExplorer"

    /** Creates (or updates the texts of) all channels; safe to call repeatedly. */
    fun ensureChannels(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        val low = NotificationManager.IMPORTANCE_LOW
        val normal = NotificationManager.IMPORTANCE_DEFAULT
        manager.createNotificationChannels(
            listOf(
                channel(context, CHANNEL_TRANSFERS, R.string.channel_transfers, R.string.channel_transfers_description, low),
                channel(context, CHANNEL_BACKGROUND, R.string.channel_background, R.string.channel_background_description, low),
                channel(context, CHANNEL_UPDATES, R.string.channel_updates, R.string.channel_updates_description, normal),
                channel(context, CHANNEL_SHARE, R.string.channel_share, R.string.channel_share_description, normal),
                execChannel(low),
            ),
        )
    }

    // Texts in code like the notifications themselves (ServiceNotifications): no string resources.
    private fun execChannel(importance: Int) =
        NotificationChannel(CHANNEL_EXEC, "Befehle anderer Geräte", importance).apply {
            description = "Solange ein anderes Gerät einen Befehl auf diesem Telefon ausführt, mit „Stopp“"
            setShowBadge(false)
        }

    /**
     * Posts [notification] when the user allows notifications; returns whether it was posted.
     * Foreground-service notifications go through `startForeground` instead.
     */
    fun notify(context: Context, id: Int, notification: Notification): Boolean {
        if (!Permissions.canPostNotifications(context)) return false
        return try {
            NotificationManagerCompat.from(context).notify(id, notification)
            true
        } catch (e: SecurityException) {
            // Permission revoked between the check and the call.
            Log.w(TAG, "notification $id not posted", e)
            false
        }
    }

    fun cancel(context: Context, id: Int) {
        NotificationManagerCompat.from(context).cancel(id)
    }

    private fun channel(context: Context, id: String, name: Int, description: Int, importance: Int) =
        NotificationChannel(id, context.getString(name), importance).apply {
            this.description = context.getString(description)
            setShowBadge(id == CHANNEL_UPDATES || id == CHANNEL_SHARE)
        }
}
