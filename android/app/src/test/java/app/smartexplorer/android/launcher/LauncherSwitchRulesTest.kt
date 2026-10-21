package app.smartexplorer.android.launcher

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** The pure decisions behind the switch and the card (U35-4, U35-8, U35-9). */
class LauncherSwitchRulesTest {
    private val own = "app.smartexplorer.android"

    @Test
    fun theAnswerToTheHomeIntentIsClassified() {
        assertEquals(HomeResolution.None, HomeRules.classify(null, null, own) { "x" })
        assertEquals(HomeResolution.Own, HomeRules.classify(own, "$own.launcher.LauncherActivity", own) { "x" })
        // Before Android 14 the selection dialog belongs to `android`, since then to `com.android.intentresolver` (A46).
        assertEquals(HomeResolution.Chooser, HomeRules.classify("android", "com.android.internal.app.ResolverActivity", own) { "x" })
        assertEquals(HomeResolution.Chooser, HomeRules.classify("com.android.intentresolver", "com.android.intentresolver.ChooserActivity", own) { "x" })
        assertEquals(
            HomeResolution.Other("com.other.home/com.other.home.Main", "Other Home"),
            HomeRules.classify("com.other.home", "com.other.home.Main", own) { "Other Home" },
        )
    }

    @Test
    fun theNoticeFollowsTheRulesOfTheCard() {
        val other = HomeResolution.Other("com.other.home/com.other.home.Main", "Other Home")
        val another = HomeResolution.Other("com.third/com.third.Main", "Third")
        val previous = "com.other.home/com.other.home.Main"
        // The previous launcher is back: its name.
        assertEquals(HomeNotice.Current("Other Home"), HomeRules.notice(previous, false, other))
        assertEquals(HomeNotice.Current("Other Home"), HomeRules.notice(previous, true, other))
        // Chooser, nothing or another launcher while the notice stands: ask again.
        assertEquals(HomeNotice.ChooseAgain(previous), HomeRules.notice(previous, false, HomeResolution.Chooser))
        assertEquals(HomeNotice.ChooseAgain(previous), HomeRules.notice(previous, false, HomeResolution.None))
        assertEquals(HomeNotice.ChooseAgain(previous), HomeRules.notice(previous, false, another))
        // Dismissed: only the name of whatever launcher answers.
        assertEquals(HomeNotice.Empty, HomeRules.notice(previous, true, HomeResolution.Chooser))
        assertEquals(HomeNotice.Current("Third"), HomeRules.notice(previous, true, another))
        // No previous launcher known: no request, the name when one answers, nothing for the chooser.
        assertEquals(HomeNotice.Empty, HomeRules.notice(null, false, HomeResolution.Chooser))
        assertEquals(HomeNotice.Current("Third"), HomeRules.notice(null, false, another))
    }

    @Test
    fun theStateFileRoundTripsAndRefusesForeignFiles() {
        val data = LauncherStateData("enabled", Parts(launcher = true, widget = false), "com.other.home/com.other.home.Main", 1234L, true)
        assertEquals(data, LauncherState.parse(LauncherState.encode(data)))
        val noPrevious = data.copy(previousHome = null)
        assertEquals(noPrevious, LauncherState.parse(LauncherState.encode(noPrevious)))
        // Unknown keys are ignored; another schema, broken JSON and unknown states count as missing.
        val withExtra = LauncherState.encode(data).replace("{\"schema\":1", "{\"future\":[1],\"schema\":1")
        assertEquals(data, LauncherState.parse(withExtra))
        assertNull(LauncherState.parse(LauncherState.encode(data).replace("\"schema\":1", "\"schema\":2")))
        assertNull(LauncherState.parse("{"))
        assertNull(LauncherState.parse("[]"))
        assertNull(LauncherState.parse(LauncherState.encode(data).replace("enabled", "sideways")))
        assertEquals(false, LauncherStateData.DISABLED.enabled)
        assertEquals(Parts(launcher = false, widget = false), LauncherStateData.DISABLED.parts)
    }
}
