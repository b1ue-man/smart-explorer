package app.smartexplorer.android.launcher.badges

import android.os.UserManager
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification

/**
 * Reads which apps have a notification that may show a dot (U38-12). It runs in the process `:launcher`, is off until
 * the launcher is switched on, and works only after the person allowed it in the system settings. It reads the package,
 * the profile and the ranking flags — never the title, text or any other content of a notification — and keeps only
 * the set of [BadgeKey]s in [Badges].
 */
class BadgeListenerService : NotificationListenerService() {
    override fun onListenerConnected() = refresh()

    override fun onListenerDisconnected() = Badges.set(emptySet())

    override fun onNotificationPosted(sbn: StatusBarNotification) = refresh()

    override fun onNotificationRemoved(sbn: StatusBarNotification) = refresh()

    override fun onNotificationRankingUpdate(rankingMap: NotificationListenerService.RankingMap) = refresh()

    private fun refresh() {
        val users = getSystemService(UserManager::class.java)
        val active = try {
            activeNotifications
        } catch (_: SecurityException) {
            null
        }
        if (active == null) {
            Badges.set(emptySet())
            return
        }
        val ranking = try {
            currentRanking
        } catch (_: SecurityException) {
            null
        }
        val reusable = NotificationListenerService.Ranking()
        val keys = HashSet<BadgeKey>()
        for (sbn in active) {
            val canShow = ranking != null && ranking.getRanking(sbn.key, reusable) && reusable.canShowBadge()
            if (BadgeRules.counts(canShow, sbn.notification.flags, sbn.isOngoing, sbn.isClearable)) {
                keys += BadgeKey(sbn.packageName, users.getSerialNumberForUser(sbn.user))
            }
        }
        Badges.set(keys)
    }
}
