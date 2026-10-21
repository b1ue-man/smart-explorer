package app.smartexplorer.android.launcher

import android.app.Notification
import android.content.Intent
import app.smartexplorer.android.launcher.apps.ProfileReceivers
import app.smartexplorer.android.launcher.badges.BadgeKey
import app.smartexplorer.android.launcher.badges.BadgeRules
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The rule for the notification dots and the profile broadcasts (U38-12 to U38-14), without Android. */
class BadgeRulesTest {
    private val summary = Notification.FLAG_GROUP_SUMMARY
    private val ongoingFlag = Notification.FLAG_ONGOING_EVENT

    @Test
    fun aNotificationCountsOnlyWhenItsChannelShowsBadgesAndItIsNeitherSummaryNorStuck() {
        assertTrue(BadgeRules.counts(canShowBadge = true, flags = 0, ongoing = false, clearable = true))
        // The channel or the app switched badges off.
        assertFalse(BadgeRules.counts(canShowBadge = false, flags = 0, ongoing = false, clearable = true))
        // The summary of a group is not counted; its members are.
        assertFalse(BadgeRules.counts(canShowBadge = true, flags = summary, ongoing = false, clearable = true))
        assertTrue(BadgeRules.counts(canShowBadge = true, flags = ongoingFlag, ongoing = false, clearable = true))
        // Foreground services and media playback: ongoing and not dismissable.
        assertFalse(BadgeRules.counts(canShowBadge = true, flags = ongoingFlag, ongoing = true, clearable = false))
        // Ongoing but the person can dismiss it: counts.
        assertTrue(BadgeRules.counts(canShowBadge = true, flags = ongoingFlag, ongoing = true, clearable = true))
    }

    @Test
    fun theDotsOfALockedPrivateSpaceAreLeftOut() {
        val keys = setOf(BadgeKey("a.one", 0), BadgeKey("a.one", 10), BadgeKey("a.two", 10), BadgeKey("a.three", 11))
        assertEquals(keys, BadgeRules.visible(keys, emptySet()))
        assertEquals(setOf(BadgeKey("a.one", 0), BadgeKey("a.three", 11)), BadgeRules.visible(keys, setOf(10L)))
        assertEquals(emptySet<BadgeKey>(), BadgeRules.visible(keys, setOf(0L, 10L, 11L)))
    }

    @Test
    fun theProfileBroadcastsDependOnTheApiLevel() {
        assertEquals(
            listOf(Intent.ACTION_MANAGED_PROFILE_AVAILABLE, Intent.ACTION_MANAGED_PROFILE_UNAVAILABLE),
            ProfileReceivers.actions(34),
        )
        assertEquals(
            listOf(
                Intent.ACTION_MANAGED_PROFILE_AVAILABLE,
                Intent.ACTION_MANAGED_PROFILE_UNAVAILABLE,
                Intent.ACTION_PROFILE_AVAILABLE,
                Intent.ACTION_PROFILE_UNAVAILABLE,
            ),
            ProfileReceivers.actions(35),
        )
    }
}
