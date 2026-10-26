package app.smartexplorer.android.ui.settings

import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.MainTab
import app.smartexplorer.android.ui.NavRequest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DeviceNavigationRegressionTest {
    @Test
    fun batches_android_device_requests_share_the_same_settings_destination() {
        val requests = listOf(
            NavRequest.OpenApps("share://direct/saved-direct"),
            NavRequest.OpenApps("share://room/saved-room/device"),
            NavRequest.OpenScreen("""{"relation":"direct","relation_id":"saved-direct"}"""),
            NavRequest.ShowScreenGrid(null), NavRequest.ShowScreenGrid("saved-room"),
            NavRequest.ShowShareRequests,
        )
        requests.forEach { request ->
            assertEquals(request.toString(), MainTab.Share, AppNav.tabFor(request))
            assertEquals(request.toString(), SettingsArea.Devices, deviceSettingsAreaFor(request))
            val (kind, value) = AppNav.encode(request)
            assertEquals("Navigation must retain the exact target", request, AppNav.decode(kind, value))
        }
    }

    @Test
    fun batches_android_non_device_requests_do_not_override_settings_navigation() {
        val requests = listOf(
            NavRequest.OpenLocation("/storage/emulated/0"),
            NavRequest.OpenLocation("share://room/saved-room/device/folder"),
            NavRequest.ShowTransfers, NavRequest.OpenJobConflicts("sync-job"),
            NavRequest.ShowUpdate, NavRequest.ShowBackgroundSettings,
            NavRequest.OpenVaultCompare("vault-job"), NavRequest.ShowLauncherSettings,
        ) + MainTab.entries.map { NavRequest.SelectTab(it) }
        requests.forEach { request -> assertNull(request.toString(), deviceSettingsAreaFor(request)) }
        assertNull(deviceSettingsAreaFor(null))
    }
}
