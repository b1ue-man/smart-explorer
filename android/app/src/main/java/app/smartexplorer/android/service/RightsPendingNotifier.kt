package app.smartexplorer.android.service

import android.app.Notification
import android.app.PendingIntent
import android.content.Context
import android.util.Log
import androidx.core.app.NotificationCompat
import app.smartexplorer.android.R
import app.smartexplorer.android.api.PendingRights
import app.smartexplorer.android.api.ShareRightsApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.system.Notifications
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.Snackbars

/** The connections in [current] that were not announced yet ([announced] holds their target keys). */
internal fun newlyPending(announced: Set<String>, current: List<PendingRights>): List<PendingRights> =
    current.filter { it.target.key !in announced }

/** Title and text of the notification for the connections in [current] that wait for their rights. */
internal fun rightsPendingText(current: List<PendingRights>): Pair<String, String> {
    val names = current.map { it.name.ifBlank { if (it.target.isRoom) "Raum" else "Gerät" } }
    val title = if (current.size == 1) "Neue Verbindung wartet auf Rechte" else "${current.size} Verbindungen warten auf Rechte"
    val shown = names.take(3).joinToString(", ") + if (names.size > 3) " und ${names.size - 3} weitere" else ""
    return title to "$shown – bis zur Bestätigung ohne Rechte. Antippen, um sie festzulegen."
}

/**
 * Tells the user that a connection arose in the background and waits for its rights (plan §F, F1).
 * Fed by [AccessApprovalNotifier], which runs on every `share` event: while the Share page is open
 * the rights sheet shows by itself; in another page of the visible app a snackbar „Festlegen“ opens
 * it; with the app hidden one notification of the channel `share` does. All three lead to the page
 * where the sheet opens. Nothing is given by tapping them.
 */
internal object RightsPendingNotifier {
    private const val TAG = "SmartExplorerRights"
    private const val CODE_OPEN_SHARE = 2006

    /** The Share page is visible: its sheet is the announcement. Set by the page's view model. */
    @Volatile
    var shareVisible: Boolean = false

    private val announced = HashSet<String>()
    private var postedFor: Set<String> = emptySet()

    suspend fun refresh(context: Context, appVisible: Boolean) {
        val current = try {
            ShareRightsApi.pending().pendingRights
        } catch (e: CoreException) {
            Log.w(TAG, "share.pendingRights failed: ${e.kind}: ${e.message}")
            return
        }
        val keys = current.map { it.target.key }.toSet()
        announced.retainAll(keys)
        val fresh = newlyPending(announced, current)
        announced += fresh.map { it.target.key }
        if (current.isEmpty() || appVisible) {
            if (postedFor.isNotEmpty()) {
                Notifications.cancel(context, Notifications.ID_RIGHTS_PENDING)
                postedFor = emptySet()
            }
        }
        if (current.isEmpty()) return
        if (appVisible) {
            if (fresh.isNotEmpty() && !shareVisible) {
                val (title, _) = rightsPendingText(fresh)
                Snackbars.show(title, "Festlegen") { AppNav.send(NavRequest.ShowShareRequests) }
            }
            return
        }
        if (keys != postedFor) {
            Notifications.ensureChannels(context)
            if (Notifications.notify(context, Notifications.ID_RIGHTS_PENDING, notification(context, current))) postedFor = keys
        }
    }

    private fun notification(context: Context, current: List<PendingRights>): Notification {
        val (title, text) = rightsPendingText(current)
        val open = PendingIntent.getActivity(
            context,
            CODE_OPEN_SHARE,
            AppNav.intentFor(context, NavRequest.ShowShareRequests),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(context, Notifications.CHANNEL_SHARE)
            .setSmallIcon(R.drawable.ic_share)
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setAutoCancel(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(open)
            .build()
    }
}
