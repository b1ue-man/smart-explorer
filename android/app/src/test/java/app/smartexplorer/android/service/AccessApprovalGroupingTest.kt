package app.smartexplorer.android.service

import app.smartexplorer.android.api.AccessApproval
import org.junit.Assert.assertEquals
import org.junit.Test

/** One notification per requesting device, announcing its number of open questions (E28, M32). */
class AccessApprovalGroupingTest {
    private fun question(number: Int, device: String) =
        AccessApproval(id = "%032x".format(number), deviceId = device, deviceName = "Gerät $device")

    @Test
    fun rueckfragenTaskTenQuestionsOfOneDeviceAreOneNotificationWithTen() {
        val groups = deviceApprovalGroups((1..10).map { question(it, "dev-a") })
        assertEquals(1, groups.size)
        assertEquals(10, groups.single().count)
    }

    @Test
    fun rueckfragenTaskQuestionsOfTwoDevicesAreTwoNotifications() {
        val questions = (1..3).map { question(it, "dev-a") } + (4..5).map { question(it, "dev-b") }
        val groups = deviceApprovalGroups(questions)
        assertEquals(listOf("dev-a", "dev-b"), groups.map { it.key })
        assertEquals(listOf(3, 2), groups.map { it.count })
    }
}
