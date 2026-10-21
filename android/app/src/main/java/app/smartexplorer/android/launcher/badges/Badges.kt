package app.smartexplorer.android.launcher.badges

import android.app.Notification
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** An app in a profile that has a notification to show a dot for. Nothing else is kept: never a title, text or icon. */
data class BadgeKey(val packageName: String, val userSerial: Long)

/** Which notifications count for a dot (U38-12), pure. */
object BadgeRules {
    /**
     * A notification counts when its channel may show badges ([canShowBadge], from the ranking), it is no group summary
     * and it is not an ongoing one the person cannot dismiss (foreground services, media playback).
     */
    fun counts(canShowBadge: Boolean, flags: Int, ongoing: Boolean, clearable: Boolean): Boolean =
        canShowBadge && (flags and Notification.FLAG_GROUP_SUMMARY) == 0 && !(ongoing && !clearable)

    /** The dots without those of profiles that are hidden, i.e. a locked private space (U38-14). */
    fun visible(keys: Set<BadgeKey>, hiddenUsers: Set<Long>): Set<BadgeKey> = keys.filterTo(HashSet()) { it.userSerial !in hiddenUsers }
}

/**
 * The dots of the home screen (U38-12), held in the process `:launcher`. [BadgeListenerService] feeds [set]; the view
 * model feeds [setHiddenUsers] with the profiles that must show nothing. The service lives in the same process, so no
 * message or storage is needed.
 */
object Badges {
    private val state = MutableStateFlow<Set<BadgeKey>>(emptySet())
    private var raw: Set<BadgeKey> = emptySet()
    private var hidden: Set<Long> = emptySet()

    /** The apps (with their profile) that currently have a counting notification, minus hidden profiles. */
    val dots: StateFlow<Set<BadgeKey>> = state.asStateFlow()

    @Synchronized
    fun set(keys: Set<BadgeKey>) {
        raw = keys
        publish()
    }

    @Synchronized
    fun setHiddenUsers(serials: Set<Long>) {
        if (serials == hidden) return
        hidden = serials
        publish()
    }

    private fun publish() {
        state.value = BadgeRules.visible(raw, hidden)
    }
}
