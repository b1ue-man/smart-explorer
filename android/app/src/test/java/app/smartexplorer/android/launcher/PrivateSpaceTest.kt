package app.smartexplorer.android.launcher

import app.smartexplorer.android.launcher.apps.PrivateSpace
import app.smartexplorer.android.launcher.apps.ProfileKind
import app.smartexplorer.android.launcher.model.GridSpec
import app.smartexplorer.android.launcher.model.HomeLayout
import app.smartexplorer.android.launcher.model.LayoutJson
import app.smartexplorer.android.launcher.model.LayoutOps
import app.smartexplorer.android.launcher.model.LayoutReadResult
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The rules of the private space (U38-14, U38-15), without Android. */
class PrivateSpaceTest {
    @Test
    fun launcher_task_private_space_settings_entry_only_api36() {
        assertFalse(PrivateSpace.settingsEntryAvailable(35))
        assertTrue(PrivateSpace.settingsEntryAvailable(36))
        assertFalse(PrivateSpace.settingsEntryAvailable(30))
        assertFalse(PrivateSpace.isSupported(34))
        assertTrue(PrivateSpace.isSupported(35))
        assertTrue(PrivateSpace.isSupported(36))
    }

    @Test
    fun theProfileExistsOnlyWithTheRoleAndThePermission() {
        assertTrue(PrivateSpace.isAvailable(35, roleHeld = true, permissionGranted = true))
        assertFalse(PrivateSpace.isAvailable(35, roleHeld = false, permissionGranted = true))
        assertFalse(PrivateSpace.isAvailable(35, roleHeld = true, permissionGranted = false))
        assertFalse(PrivateSpace.isAvailable(34, roleHeld = true, permissionGranted = true))
    }

    @Test
    fun aLockedPrivateSpaceShowsNothingAndAnUnlockedOneOnlyWhenWanted() {
        assertTrue(PrivateSpace.isHidden(ProfileKind.Private, quiet = true))
        assertFalse(PrivateSpace.isHidden(ProfileKind.Private, quiet = false))
        // A paused work profile is not hidden: its apps stay in the list, greyed (U38-13).
        assertFalse(PrivateSpace.isHidden(ProfileKind.Work, quiet = true))
        assertTrue(PrivateSpace.appVisible(ProfileKind.Personal, quiet = false, showSetting = false))
        assertTrue(PrivateSpace.appVisible(ProfileKind.Work, quiet = true, showSetting = false))
        assertTrue(PrivateSpace.appVisible(ProfileKind.Private, quiet = false, showSetting = true))
        assertFalse(PrivateSpace.appVisible(ProfileKind.Private, quiet = false, showSetting = false))
        assertFalse(PrivateSpace.appVisible(ProfileKind.Private, quiet = true, showSetting = true))
        assertFalse(PrivateSpace.appVisible(ProfileKind.Private, quiet = true, showSetting = false))
    }

    @Test
    fun theLineOfTheCardFollowsTheStateOfTheDevice() {
        assertEquals("Privater Bereich: nicht verfügbar (Android 15 oder neuer nötig)", PrivateSpace.statusLine(34, roleHeld = true, permissionGranted = true))
        assertEquals("Privater Bereich: erst als Standard-Startbildschirm verfügbar", PrivateSpace.statusLine(35, roleHeld = false, permissionGranted = true))
        assertEquals("Privater Bereich: Berechtigung fehlt", PrivateSpace.statusLine(35, roleHeld = true, permissionGranted = false))
        assertEquals("Privater Bereich: verfügbar", PrivateSpace.statusLine(36, roleHeld = true, permissionGranted = true))
    }

    @Test
    fun theSwitchOfTheSheetIsStoredWithoutTouchingTheRest() {
        val layout = HomeLayout(GridSpec(4, 5, 56), listOf(emptyList()), emptyList(), JsonObject(mapOf("seSeeded" to JsonPrimitive(true))))
        // Missing means on (U38-14).
        assertTrue(LayoutOps.showPrivateSpace(layout))
        val off = LayoutOps.withShowPrivateSpace(layout, false)
        assertFalse(LayoutOps.showPrivateSpace(off))
        assertEquals(JsonPrimitive(true), off.extra["seSeeded"])
        val reread = (LayoutJson.parse(LayoutJson.encode(off)) as LayoutReadResult.Loaded).layout
        assertFalse(LayoutOps.showPrivateSpace(reread))
        assertTrue(LayoutOps.showPrivateSpace(LayoutOps.withShowPrivateSpace(reread, true)))
        // A foreign value in the file counts as on.
        val foreign = layout.copy(extra = JsonObject(mapOf("showPrivateSpace" to JsonPrimitive("nein"))))
        assertTrue(LayoutOps.showPrivateSpace(foreign))
    }
}
