package app.smartexplorer.android.launcher

import app.smartexplorer.android.api.DocsSettings
import app.smartexplorer.android.api.ScreenDevice
import app.smartexplorer.android.api.ShareDevice
import app.smartexplorer.android.api.ShareMember
import app.smartexplorer.android.api.ShareRoom
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.api.VaultGroup
import app.smartexplorer.android.api.VaultList
import app.smartexplorer.android.api.VaultSite
import app.smartexplorer.android.launcher.bridge.BridgeProtocol
import app.smartexplorer.android.launcher.bridge.DevicesData
import app.smartexplorer.android.launcher.bridge.DevicesFolder
import app.smartexplorer.android.launcher.bridge.DevicesJson
import app.smartexplorer.android.launcher.bridge.GlobalResult
import app.smartexplorer.android.launcher.bridge.ResultJson
import app.smartexplorer.android.launcher.bridge.ScreenRef
import app.smartexplorer.android.launcher.bridge.StatusCodes
import app.smartexplorer.android.launcher.se.DeviceKey
import app.smartexplorer.android.service.LauncherSnapshot
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * DSK-M13, the bridge on the JVM without Android (U40-4, U40-5, U40-14, U40-21): the device key, the status codes of
 * the room members, the shape of `DEVICES` and the cache form, and the snapshot built from the answers of the core.
 */
class SeBridgeTest {
    @Test
    fun launcher_task_device_key_vectors() {
        assertEquals("direct~c1", DeviceKey.direct("c1"))
        assertEquals("direct~a%20b%2F%C3%BC", DeviceKey.direct("a b/ü"))
        assertEquals("room~r%7E1~d.e-f_g", DeviceKey.room("r~1", "d.e-f_g"))
        assertEquals("direct~c1", DeviceKey.of("share://direct/c1/"))
        assertEquals("direct~c1", DeviceKey.of("share://direct/c1"))
        assertEquals("room~r1~d1", DeviceKey.of("share://room/r1/d1"))
        assertNull(DeviceKey.of("share://x/y"))
        assertNull(DeviceKey.of("share://direct/"))
        assertNull(DeviceKey.of("share://room/r1"))
    }

    @Test
    fun launcher_task_status_label_codes() {
        val expected = listOf(
            "Offline" to "offline",
            "Wartet" to "waiting",
            "Warte auf Freigabe" to "waitingForAccess",
            "Online" to "available",
            "Verbinde" to "connecting",
            "Verbunden" to "connected",
            "Direkt verbunden" to "connectedDirect",
            "Relay verbunden" to "connectedRelay",
            "Fehler: keine Antwort" to "failed",
            "Identitaetskonflikt" to "identityConflict",
            "irgendein neuer Text" to "unknown",
            "" to "unknown",
        )
        for ((label, code) in expected) assertEquals("label \"$label\"", code, StatusCodes.fromLabel(label))
        // online = code in {available, connected, connectedDirect, connectedRelay}, like `device_view::reachable`.
        val online = listOf("available", "connected", "connectedDirect", "connectedRelay")
        for ((_, code) in expected) assertEquals(code, code in online, StatusCodes.isOnline(code))
        assertEquals("keine Antwort", StatusCodes.statusTextOf("Fehler: keine Antwort"))
        assertEquals("Verbunden", StatusCodes.statusTextOf("Verbunden"))
        assertEquals("keine Antwort", StatusCodes.errorDetail("Fehler: keine Antwort"))
        assertEquals("keine Antwort", StatusCodes.errorDetail("keine Antwort"))
    }

    private fun sampleStatus() = ShareStatus(
        devices = listOf(
            ShareDevice(
                contactId = "c1", name = "Laptop", status = "connectedDirect", statusText = "Direkt verbunden", online = true,
                location = "share://direct/c1", lan = true, shareBack = true, write = false, autoConnect = true, autoOpen = false,
            ),
        ),
        rooms = listOf(
            ShareRoom(
                profileId = "p1", roomId = "ro1", name = "Team", status = "Verbunden",
                members = listOf(
                    ShareMember(deviceId = "d1", name = "Desktop", status = "Relay verbunden", location = "share://room/p1/d1", publicKey = "SECRET", fingerprint = "FP"),
                    ShareMember(deviceId = "d2", name = "Tablet", status = "Fehler: weg", location = "share://room/p1/d2", blocked = true),
                ),
            ),
        ),
    )

    @Test
    fun the_snapshot_keeps_only_the_fields_of_the_bridge() {
        val screens = listOf(
            ScreenDevice(relation = "direct", relationId = "c1", deviceId = "dev-c1"),
            ScreenDevice(relation = "room", relationId = "p1", deviceId = "d1"),
            ScreenDevice(relation = "room", relationId = "other", deviceId = "d2"),
        )
        val vaults = VaultList(listOf(VaultGroup("v", listOf(VaultSite("s1", "Privat", "/geheim/pfad", "local", state = "unlocked")))))
        val docs = DocsSettings(provider = true, devices = true)
        val data = LauncherSnapshot.assemble(sampleStatus(), vaults, screens, docs, "app.documents")

        val device = data.devices.single()
        assertEquals("direct~c1", device.key)
        assertEquals(ScreenRef("direct", "c1", "dev-c1"), device.screen)
        assertEquals("share://direct/c1", device.location)
        assertEquals("connectedDirect", device.status)
        assertTrue(device.lan)

        val room = data.rooms.single()
        assertEquals("p1", room.profileId)
        val desktop = room.members[0]
        assertEquals("connectedRelay", desktop.status)
        assertEquals("Relay verbunden", desktop.statusText)
        assertTrue(desktop.online)
        assertEquals("room~p1~d1", desktop.key)
        assertEquals(ScreenRef("room", "p1", "d1"), desktop.screen)
        val tablet = room.members[1]
        assertEquals("failed", tablet.status)
        assertEquals("weg", tablet.statusText)
        assertFalse(tablet.online)
        assertTrue(tablet.blocked)
        // The screen row of another room does not count for this member.
        assertNull(tablet.screen)

        assertEquals(listOf("s1"), data.vaults.map { it.site })
        assertEquals("Privat", data.vaults.single().label)
        assertEquals("unlocked", data.vaults.single().state)
        assertEquals(DevicesFolder("app.documents", "devices"), data.devicesFolder)

        // Nothing of the rooms' keys or of a vault's location reaches the launcher.
        val text = DevicesJson.encode(data, withToken = true)
        assertFalse(text.contains("SECRET"))
        assertFalse(text.contains("FP"))
        assertFalse(text.contains("/geheim/pfad"))
    }

    @Test
    fun a_failing_source_leaves_its_part_empty() {
        val data = LauncherSnapshot.assemble(sampleStatus(), null, null, null, "app.documents")
        assertEquals(emptyList<Any>(), data.vaults)
        assertNull(data.devices.single().screen)
        assertNull(data.devicesFolder)
        val off = LauncherSnapshot.assemble(sampleStatus(), null, null, DocsSettings(provider = true, devices = false), "a")
        assertNull(off.devicesFolder)
    }

    @Test
    fun devices_json_has_the_documented_shape() {
        val data = LauncherSnapshot.assemble(
            sampleStatus(),
            VaultList(listOf(VaultGroup(null, listOf(VaultSite("s1", "Privat"))))),
            listOf(ScreenDevice("direct", "c1", "dev-c1")),
            DocsSettings(provider = true, devices = true),
            "app.documents",
        )
        val text = LauncherSnapshot.toJson(LauncherSnapshot(data), "tok")
        val root = Json.parseToJsonElement(text) as JsonObject
        assertEquals(JsonPrimitive(1), root["v"])
        assertEquals(JsonPrimitive("tok"), root["navToken"])
        assertEquals(JsonPrimitive(false), root["stale"])
        val device = (root["devices"] as JsonArray)[0] as JsonObject
        // The eleven keys of share.status plus key and screen: thirteen.
        assertEquals(
            setOf(
                "contactId", "name", "status", "statusText", "online", "location", "lan", "shareBack", "write", "autoConnect", "autoOpen",
                "key", "screen",
            ),
            device.keys,
        )
        val member = (((root["rooms"] as JsonArray)[0] as JsonObject)["members"] as JsonArray)[0] as JsonObject
        assertEquals(setOf("deviceId", "name", "status", "statusText", "online", "location", "blocked", "key", "screen"), member.keys)
        assertEquals(setOf("site", "label", "state"), ((root["vaults"] as JsonArray)[0] as JsonObject).keys)
        assertEquals(setOf("authority", "rootId"), (root["devicesFolder"] as JsonObject).keys)
        assertEquals(setOf("relation", "relationId", "deviceId"), (device["screen"] as JsonObject).keys)

        val parsed = DevicesJson.parse(text)!!
        assertEquals("tok", parsed.navToken)
        assertEquals(data.copy(navToken = "tok"), parsed)
    }

    @Test
    fun the_cache_form_has_no_token_and_is_stale() {
        val data = DevicesData(navToken = "tok", devices = emptyList(), rooms = emptyList())
        val cache = DevicesJson.encode(data, withToken = false)
        val root = Json.parseToJsonElement(cache) as JsonObject
        assertFalse(root.containsKey("navToken"))
        assertEquals(JsonPrimitive(true), root["stale"])
        assertFalse(cache.contains("tok"))
        val read = DevicesJson.parse(cache)!!
        assertNull(read.navToken)
        assertTrue(read.stale)
    }

    @Test
    fun error_messages_and_results_round_trip() {
        val error = DevicesJson.encodeError(BridgeProtocol.ERROR_TOO_LARGE)
        assertEquals("""{"v":1,"error":"Zu viele Einträge für die Übertragung"}""", error)
        assertEquals(BridgeProtocol.ERROR_TOO_LARGE, DevicesJson.parse(error)!!.error)
        assertNull(DevicesJson.parse("""{"v":2,"devices":[]}"""))
        assertNull(DevicesJson.parse("kein json"))
        for (result in listOf(GlobalResult(true), GlobalResult(false, "accessibility_off"), GlobalResult(false, "unsupported"))) {
            assertEquals(result, ResultJson.parse(ResultJson.encode(result)))
        }
        assertEquals("""{"ok":true,"reason":null}""", ResultJson.encode(GlobalResult(true)))
        assertNotNull(Json.parseToJsonElement(ResultJson.encode(GlobalResult(false, "unsupported"))))
        assertEquals(JsonNull, (Json.parseToJsonElement(ResultJson.encode(GlobalResult(true))) as JsonObject)["reason"])
    }
}
