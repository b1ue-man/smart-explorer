package app.smartexplorer.android.ui.share

import app.smartexplorer.android.api.TcpTarget
import app.smartexplorer.android.ui.screen.ScreenTarget
import app.smartexplorer.android.ui.screen.ScreenTargetJson
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** WP3: „Bildschirm steuern“ next to a locked host session opens that device's screen with control. */
class AppsLockedScreenTest {
    private val room = TcpTarget(
        relation = "room", relationId = "raum-1", deviceId = "geraet-7", publicKey = "pk",
        nodeId = "node-7", name = "Laptop", relationLabel = "Raum",
    )

    @Test
    fun lockedSessionOpensTheDeviceScreenWithControlAndPinnedKey() {
        val expected = ScreenTarget("room", "raum-1", "geraet-7", control = true, expectedNodeId = "node-7")
        assertEquals(expected, lockedSessionScreen(room, locked = true, sessionUnlocked = null))
        assertEquals(expected, lockedSessionScreen(room, locked = false, sessionUnlocked = false))
        // The existing screen-open flow (NavRequest.OpenScreen) carries the same target.
        assertEquals(expected, ScreenTargetJson.parse(expected.toJson()).getOrThrow())
    }

    @Test
    fun noButtonWhileUnlockedOrWithoutDevice() {
        assertNull(lockedSessionScreen(room, locked = false, sessionUnlocked = true))
        assertNull(lockedSessionScreen(room, locked = false, sessionUnlocked = null))
        assertNull(lockedSessionScreen(null, locked = true, sessionUnlocked = false))
    }

    @Test
    fun directDeviceWithoutIdsKeepsThemUnset() {
        val direct = room.copy(relation = "direct", relationId = "kontakt", deviceId = "", nodeId = "")
        assertEquals(
            ScreenTarget("direct", "kontakt", null, control = true, expectedNodeId = null),
            lockedSessionScreen(direct, locked = true, sessionUnlocked = false),
        )
    }
}
