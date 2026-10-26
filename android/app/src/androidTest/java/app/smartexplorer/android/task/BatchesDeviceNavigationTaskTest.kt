package app.smartexplorer.android.task

import androidx.activity.compose.setContent
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.test.assertDoesNotExist
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.lifecycle.Lifecycle
import app.smartexplorer.android.MainActivity
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.DeviceSettingsRoot
import app.smartexplorer.android.ui.NavRequest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Real settings composition and authenticated onNewIntent delivery on the existing Activity. */
@RunWith(AndroidJUnit4::class)
class BatchesDeviceNavigationTaskTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()

    private fun root() {
        compose.runOnUiThread {
            AppNav.pending.value?.let(AppNav::consume)
            // Remove already delivered queue entries before replacing the activity content.
            AppNav.flushQueued()
            compose.activity.setContent { MaterialTheme { DeviceSettingsRoot(onBack = {}) } }
        }
        compose.waitForIdle()
    }

    @After
    fun cleanup() {
        compose.runOnUiThread {
            compose.activity.setContent { }
            AppNav.pending.value?.let(AppNav::consume)
            AppNav.flushQueued()
        }
    }

    private fun selectArea(label: String) {
        val selector = compose.onNodeWithText("Bereich: ", substring = true)
        selector.performClick()
        compose.onNodeWithText(label, useUnmergedTree = true).performClick()
        compose.onNodeWithText("Bereich: $label").assertIsDisplayed()
    }

    private fun detail(label: String) {
        selectArea("Diagnose und Protokolle")
        compose.onNodeWithText(label).performScrollTo().performClick()
        if (label == "Über") compose.onNodeWithText("Smart Explorer für Android").assertIsDisplayed()
        else compose.onNodeWithText("Fehlerprotokoll").assertIsDisplayed()
    }

    private fun deliver(request: NavRequest) {
        val existing = compose.activity
        compose.runOnUiThread { existing.startActivity(AppNav.intentFor(existing, request)) }
        compose.waitForIdle()
        assertSame("A warm navigation request must reuse the existing activity", existing, compose.activity)
        compose.runOnIdle { assertEquals(Lifecycle.State.RESUMED, existing.lifecycle.currentState) }
    }

    private fun waitForText(text: String) {
        compose.waitUntil(10_000) { compose.onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText(text).assertIsDisplayed()
    }

    private fun assertAppsOnce() {
        waitForText("Programme · Gerät")
        compose.runOnIdle { assertNull("Only the visible target consumes the request", AppNav.pending.value) }
        compose.onNodeWithText("Schließen").performClick()
        compose.onNodeWithText("Programme · Gerät").assertDoesNotExist()
        selectArea("Darstellung")
        selectArea("Geräte")
        compose.onNodeWithText("Programme · Gerät").assertDoesNotExist()
    }

    @Test
    fun batches_android_apps_request_from_existing_appearance_opens_once() {
        root()
        selectArea("Darstellung")
        deliver(NavRequest.OpenApps("share://direct/device-navigation-fixture"))
        assertAppsOnce()
    }

    @Test
    fun batches_android_apps_request_leaves_log_and_about_details() {
        for (page in listOf("Fehlerprotokoll", "Über")) {
            root()
            detail(page)
            deliver(NavRequest.OpenApps("share://room/device-navigation-room/device-navigation-fixture"))
            assertAppsOnce()
        }
    }

    @Test
    fun batches_android_untrusted_device_intent_preserves_the_existing_detail() {
        root()
        detail("Über")
        val request = NavRequest.OpenApps("share://direct/device-navigation-fixture")
        compose.runOnUiThread {
            val activity = compose.activity
            activity.startActivity(AppNav.intentFor(activity, request, "untrusted-device-navigation-token"))
        }
        compose.waitForIdle()
        compose.onNodeWithText("Smart Explorer für Android").assertIsDisplayed()
        compose.runOnIdle { assertNull(AppNav.pending.value) }
        deliver(request)
        assertAppsOnce()
    }

    @Test
    fun batches_android_screen_and_grid_requests_leave_log_and_about_details() {
        val requests = listOf(
            NavRequest.OpenScreen("""{"relation":"direct","relation_id":"device-navigation-fixture","device_id":null,"source":null,"control":false}"""),
            NavRequest.ShowScreenGrid(null),
        )
        for (page in listOf("Fehlerprotokoll", "Über")) for (request in requests) {
            root()
            detail(page)
            deliver(request)
            val close = if (request is NavRequest.OpenScreen) "screen_view_close" else "screen_grid_close"
            compose.onNodeWithTag(close).assertIsDisplayed()
            compose.runOnIdle { assertNull(AppNav.pending.value) }
            compose.onNodeWithTag(close).performClick()
            compose.onNodeWithTag(close).assertDoesNotExist()
            compose.onNodeWithText("Bereich: Geräte").assertIsDisplayed()
        }
    }

    @Test
    fun batches_android_share_requests_leave_details_and_unrelated_requests_keep_them() {
        for (page in listOf("Fehlerprotokoll", "Über")) {
            root()
            detail(page)
            val unrelated = NavRequest.OpenLocation("/storage/emulated/0")
            // This root intentionally has no Files tab consumer; it must leave both state and request alone.
            compose.runOnUiThread { AppNav.send(unrelated) }
            compose.waitForIdle()
            if (page == "Über") compose.onNodeWithText("Smart Explorer für Android").assertIsDisplayed()
            else compose.onNodeWithText("Fehlerprotokoll").assertIsDisplayed()
            compose.runOnIdle { assertEquals(unrelated, AppNav.pending.value) }
            deliver(NavRequest.ShowShareRequests)
            compose.onNodeWithText("Bereich: Geräte").assertIsDisplayed()
            compose.waitUntil(10_000) { AppNav.pending.value == null }
        }
    }
}
