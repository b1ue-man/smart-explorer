package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The rights contract of a new connection (api.md §5.1, plan §F, package G3): the answers decoded
 * with `Core.json` and the arguments of `share.confirmRights`, `share.cancelRights`,
 * `share.setCodeRights`, `share.decide`, `share.discoverable`, `share.createRoom` and
 * `share.setPolicy`.
 */
class AbgleichTaskRightsApiTest {
    private val direct = RightsTarget.direct("dev-1", "pk-1")

    @Test
    fun abgleichTaskStatusCarriesPendingRightsAndTheCodeRights() {
        val status = Core.json.decodeFromString(
            ShareStatus.serializer(),
            """{"running":true,"identity":{},"discovery":{},
               "pendingRights":[
                 {"target":{"kind":"direct","deviceId":"d1","publicKey":"k1"},"origin":"request","since":1790000000,"name":"Laptop",
                  "rights":{"folders":{"kind":"none"},"write":false,"commands":"off","terminal":"off","screenView":"off",
                            "screenControl":"off","validUntil":null,"account":"own","preset":"none","summary":"Keine Rechte"}},
                 {"target":{"kind":"room","profileId":"p9"},"origin":"room_created","since":1790000100,"name":"Team",
                  "rights":{"folders":{"kind":"selected","paths":["/a","/b"],"connections":true},"write":true,"commands":"ask",
                            "terminal":"off","screenView":"allow","screenControl":"off","validUntil":1790003600,"account":"own"}}],
               "codeRights":{"folders":{"kind":"all"},"write":true,"commands":"off","terminal":"off","screenView":"off",
                             "screenControl":"off","account":"own","preset":"read-write","summary":"Dateien lesen und schreiben"},
               "incoming":[{"requestId":"r1","name":"Laptop","canAccept":true,"fingerprint":"AA:BB","deviceId":"d1"}]}""",
        )
        val (device, room) = status.pendingRights
        assertEquals(direct.copy(deviceId = "d1", publicKey = "k1"), device.target)
        assertEquals("direct/d1/k1", device.target.key)
        assertEquals("request", device.origin)
        assertEquals("none", device.rights.preset)
        assertTrue(room.target.isRoom)
        assertEquals("room/p9", room.target.key)
        assertEquals(listOf("/a", "/b"), room.rights.folders.paths)
        assertEquals(true, room.rights.folders.connections)
        assertEquals(ACCESS_ASK, room.rights.commands)
        assertEquals(1790003600L, room.rights.validUntil)
        assertEquals(FOLDERS_ALL, status.codeRights?.folders?.kind)
        assertEquals("AA:BB", status.incoming.single().fingerprint)
        assertEquals("d1", status.incoming.single().deviceId)
    }

    @Test
    fun abgleichTaskOlderBridgeWithoutTheNewFieldsDecodesToNoPendingRights() {
        val status = Core.json.decodeFromString(ShareStatus.serializer(), """{"identity":{},"discovery":{},"incoming":[{"requestId":"r"}]}""")
        assertTrue(status.pendingRights.isEmpty())
        assertNull(status.codeRights)
        assertNull(status.incoming.single().fingerprint)
        val policy = Core.json.decodeFromString(ShareRequestPolicy.serializer(), """{"requests":"AutoAccept"}""")
        assertNull(policy.codeRights)
        val withRights = Core.json.decodeFromString(
            ShareRequestPolicy.serializer(),
            """{"requests":"AutoAccept","codeRights":{"folders":{"kind":"none"},"write":false,"commands":"allow"}}""",
        )
        assertEquals(ACCESS_ALLOW, withRights.codeRights?.commands)
    }

    @Test
    fun abgleichTaskPresetsDecodeWithKeyLabelAndRights() {
        val presets = Core.json.decodeFromString(
            PendingRightsAnswer.serializer(),
            """{"pendingRights":[],"codeRights":null}""",
        )
        assertTrue(presets.pendingRights.isEmpty())
        assertNull(presets.codeRights)
        val info = Core.json.decodeFromString(
            RightsPresetInfo.serializer(),
            """{"key":"read-only","label":"Nur lesen","description":"d","rights":{"folders":{"kind":"all"},"write":false}}""",
        )
        assertEquals(PRESET_READ_ONLY, info.key)
        assertEquals(FOLDERS_ALL, info.rights.folders.kind)
        assertFalse(info.rights.write)
    }

    @Test
    fun abgleichTaskConfirmRightsSendsTheTargetAndThePresetPlusChangedFields() {
        val selection = RightsSelection(
            PRESET_READ_ONLY,
            rightsFieldsJson(Rights(write = true), listOf(RIGHTS_FIELD_WRITE)),
        )
        val args = confirmRightsArgs(direct, selection)
        val target = args["target"]!!.jsonObject
        assertEquals("direct", target["kind"]!!.jsonPrimitive.content)
        assertEquals("dev-1", target["deviceId"]!!.jsonPrimitive.content)
        assertEquals("pk-1", target["publicKey"]!!.jsonPrimitive.content)
        assertEquals("read-only", args["preset"]!!.jsonPrimitive.content)
        assertTrue(args["rights"]!!.jsonObject["write"]!!.jsonPrimitive.boolean)
        assertEquals(setOf("target", "preset", "rights"), args.keys)
    }

    @Test
    fun abgleichTaskConfirmRightsForARoomNamesTheProfile() {
        val args = confirmRightsArgs(RightsTarget.room("p9"), RightsSelection.preset(PRESET_NONE))
        val target = args["target"]!!.jsonObject
        assertEquals("room", target["kind"]!!.jsonPrimitive.content)
        assertEquals("p9", target["profileId"]!!.jsonPrimitive.content)
        assertFalse("deviceId" in target)
        assertEquals("none", args["preset"]!!.jsonPrimitive.content)
        assertFalse("rights" in args)
    }

    @Test
    fun abgleichTaskConfirmRightsWithoutPresetAndFieldsIsRefusedBeforeTheCall() {
        val failure = assertThrows(CoreException::class.java) { confirmRightsArgs(direct, RightsSelection()) }
        assertEquals("invalid", failure.kind)
        assertThrows(CoreException::class.java) { confirmRightsArgs(direct, RightsSelection(rights = JsonObject(emptyMap()))) }
    }

    @Test
    fun abgleichTaskCancelRightsNamesOnlyTheTarget() {
        val args = cancelRightsArgs(direct)
        assertEquals(setOf("target"), args.keys)
        assertEquals("pk-1", args["target"]!!.jsonObject["publicKey"]!!.jsonPrimitive.content)
    }

    @Test
    fun abgleichTaskCodeRightsSetOrClear() {
        val set = codeRightsArgs(RightsSelection.preset(PRESET_FILES_EXEC), clear = false)
        assertEquals("files-exec", set["preset"]!!.jsonPrimitive.content)
        assertFalse("clear" in set)
        val clear = codeRightsArgs(null, clear = true)
        assertTrue(clear["clear"]!!.jsonPrimitive.boolean)
        assertFalse("preset" in clear)
        assertThrows(CoreException::class.java) { codeRightsArgs(null, clear = false) }
        assertThrows(CoreException::class.java) { codeRightsArgs(RightsSelection(), clear = false) }
    }

    @Test
    fun abgleichTaskRightsJsonSendsTheFolderSelectionAndRemovesTheEndWithNull() {
        val rights = Rights(
            folders = RightsFolders(FOLDERS_SELECTED, listOf("/x"), connections = null),
            write = true,
            commands = ACCESS_ALLOW,
            screenView = ACCESS_ASK,
        )
        val json = rightsFieldsJson(rights)
        val folders = json["folders"]!!.jsonObject
        assertEquals("selected", folders["kind"]!!.jsonPrimitive.content)
        assertEquals(listOf("/x"), folders["paths"]!!.let { (it as kotlinx.serialization.json.JsonArray).map { p -> p.jsonPrimitive.content } })
        assertFalse(folders["connections"]!!.jsonPrimitive.boolean)
        assertEquals("allow", json["commands"]!!.jsonPrimitive.content)
        assertEquals("ask", json["screenView"]!!.jsonPrimitive.content)
        assertEquals("off", json["terminal"]!!.jsonPrimitive.content)
        assertEquals("own", json["account"]!!.jsonPrimitive.content)
        assertTrue(json["validUntil"] is kotlinx.serialization.json.JsonNull)
        val onlyEnd = rightsFieldsJson(rights.copy(validUntil = 1790000000), listOf(RIGHTS_FIELD_VALID_UNTIL))
        assertEquals(setOf("validUntil"), onlyEnd.keys)
        assertEquals(1790000000L, (onlyEnd["validUntil"] as JsonPrimitive).content.toLong())
    }

    @Test
    fun abgleichTaskAcceptingCarriesTheRightsRejectingNever() {
        val rights = RightsSelection.preset(PRESET_READ_WRITE)
        val accept = decideArgs("r1", true, rights)
        assertEquals("read-write", accept["preset"]!!.jsonPrimitive.content)
        assertTrue(accept["accept"]!!.jsonPrimitive.boolean)
        val reject = decideArgs("r1", false, rights)
        assertFalse("preset" in reject)
        assertFalse("rights" in reject)
        val plain = decideArgs("r1", true, null)
        assertEquals(setOf("requestId", "accept"), plain.keys)
    }

    @Test
    fun abgleichTaskPinOfferRoomAndAutoAcceptCarryTheirRights() {
        val rights = RightsSelection.preset(PRESET_ALL)
        val offer = discoverableArgs("direct", "Handy", "123987", 5, false, rights)
        assertEquals("all", offer["preset"]!!.jsonPrimitive.content)
        assertEquals("direct", offer["target"]!!.jsonPrimitive.content)
        assertFalse("preset" in discoverableArgs("direct", "Handy", "123987", 5, false, null))
        val room = createRoomArgs("Team", RightsSelection.preset(PRESET_READ_ONLY))
        assertEquals("Team", room["name"]!!.jsonPrimitive.content)
        assertEquals("read-only", room["preset"]!!.jsonPrimitive.content)
        assertEquals(setOf("name"), createRoomArgs("Team", null).keys)
        val policy = setPolicyArgs("AutoAccept", RightsSelection.preset(PRESET_NONE))
        assertEquals("AutoAccept", policy["requests"]!!.jsonPrimitive.content)
        assertEquals("none", policy["preset"]!!.jsonPrimitive.content)
        assertEquals(setOf("requests"), setPolicyArgs("Ask", null).keys)
    }

    @Test
    fun abgleichTaskChangeAnswerIsDecoded() {
        val change = Core.json.decodeFromString(
            RightsChange.serializer(),
            """{"changed":true,"persisted":true,"pendingRights":[],"codeRights":null}""",
        )
        assertTrue(change.changed)
        assertTrue(change.persisted)
        assertTrue(change.pendingRights.isEmpty())
    }
}
