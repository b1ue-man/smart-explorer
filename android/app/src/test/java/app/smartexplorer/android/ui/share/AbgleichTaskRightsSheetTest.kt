package app.smartexplorer.android.ui.share

import app.smartexplorer.android.api.ACCESS_ALLOW
import app.smartexplorer.android.api.ACCESS_ASK
import app.smartexplorer.android.api.ACCESS_OFF
import app.smartexplorer.android.api.AccessProviderInfo
import app.smartexplorer.android.api.CAP_COMMANDS
import app.smartexplorer.android.api.CAP_SCREEN_CONTROL
import app.smartexplorer.android.api.CAP_SCREEN_VIEW
import app.smartexplorer.android.api.CAP_TERMINAL
import app.smartexplorer.android.api.CapabilityRow
import app.smartexplorer.android.api.ExecProviderInfo
import app.smartexplorer.android.api.FOLDERS_ALL
import app.smartexplorer.android.api.FOLDERS_NONE
import app.smartexplorer.android.api.FOLDERS_SELECTED
import app.smartexplorer.android.api.PRESET_ALL
import app.smartexplorer.android.api.PRESET_FILES_EXEC
import app.smartexplorer.android.api.PRESET_NONE
import app.smartexplorer.android.api.PRESET_READ_ONLY
import app.smartexplorer.android.api.PRESET_READ_WRITE
import app.smartexplorer.android.api.PendingRights
import app.smartexplorer.android.api.RIGHTS_FIELD_COMMANDS
import app.smartexplorer.android.api.RIGHTS_FIELD_FOLDERS
import app.smartexplorer.android.api.RIGHTS_FIELD_SCREEN_CONTROL
import app.smartexplorer.android.api.RIGHTS_FIELD_SCREEN_VIEW
import app.smartexplorer.android.api.RIGHTS_FIELD_WRITE
import app.smartexplorer.android.api.Rights
import app.smartexplorer.android.api.RightsFolders
import app.smartexplorer.android.api.RightsTarget
import app.smartexplorer.android.api.ShareDevice
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.api.ShareWriteGrant
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The compact rights menu of a new connection (plan 2026-10-31 §F, package G3): the presets and how
 * they fill the draft, the state of the sheet (what waits, what was put off, what „Rechte
 * einstellen“ does not guess), what „Bestätigen“ and „Abbrechen“ call and the texts that describe it.
 */
class AbgleichTaskRightsSheetTest {
    private val presets = RightsPresets.builtIn()
    private val everything = OfferedRights()
    private val device = RightsTarget.direct("d1", "k1")
    private val room = RightsTarget.room("p1")
    private fun preset(key: String) = presets.first { it.key == key }
    private fun pending(target: RightsTarget, name: String = "Laptop", rights: Rights = Rights()) =
        PendingRights(target, origin = "request", since = 1, name = name, rights = rights)

    @Test
    fun abgleichTaskThereAreFivePresetsWithTheContractKeysAndLabels() {
        assertEquals(listOf("none", "read-only", "read-write", "files-exec", "all"), presets.map { it.key })
        assertEquals(
            listOf("Keine Rechte", "Nur lesen", "Dateien lesen und schreiben", "Dateien und Codeausführung", "Alles"),
            presets.map { it.label },
        )
    }

    @Test
    fun abgleichTaskEachPresetFillsTheDraftAsTheContractSays() {
        val none = preset(PRESET_NONE).rights
        assertEquals(FOLDERS_NONE, none.folders.kind)
        assertFalse(none.write)
        val readOnly = preset(PRESET_READ_ONLY).rights
        assertEquals(FOLDERS_ALL, readOnly.folders.kind)
        assertFalse(readOnly.write)
        assertEquals(ACCESS_OFF, readOnly.commands)
        val readWrite = preset(PRESET_READ_WRITE).rights
        assertTrue(readWrite.write)
        assertEquals(ACCESS_OFF, readWrite.terminal)
        val filesExec = preset(PRESET_FILES_EXEC).rights
        assertTrue(filesExec.write)
        assertEquals(ACCESS_ALLOW, filesExec.commands)
        assertEquals(ACCESS_ALLOW, filesExec.terminal)
        assertEquals(ACCESS_OFF, filesExec.screenView)
        val all = preset(PRESET_ALL).rights
        assertEquals(ACCESS_ALLOW, all.screenView)
        assertEquals(ACCESS_ALLOW, all.screenControl)
        presets.forEach { info ->
            val draft = RightsDraft().applyPreset(info, everything)
            assertEquals(info.key, draft.matchingPreset(presets)?.key)
            // A preset on its own is sent as its key, nothing else.
            val selection = draft.selection(presets, 1_000L)
            assertEquals(info.key, selection.preset)
            assertNull(selection.rights)
        }
    }

    @Test
    fun abgleichTaskAChangedRowLeavesThePresetChipAndIsSentAsTheOnlyDifference() {
        val draft = RightsDraft().applyPreset(preset(PRESET_READ_ONLY), everything).setWrite(true)
        assertEquals(PRESET_READ_WRITE, draft.matchingPreset(presets)?.key)
        val selection = draft.selection(presets, 1_000L)
        assertEquals(PRESET_READ_ONLY, selection.preset)
        assertEquals(setOf("write"), selection.rights!!.keys)
        assertTrue(selection.rights!!["write"]!!.jsonPrimitive.boolean)
        val custom = RightsDraft().applyPreset(preset(PRESET_READ_ONLY), everything).setDecision(RIGHTS_FIELD_COMMANDS, ACCESS_ASK)
        assertNull(custom.matchingPreset(presets))
    }

    @Test
    fun abgleichTaskARightThePhoneCannotGiveIsTurnedOffAndSentOff() {
        val noScreen = OfferedRights(screenView = false, screenControl = false)
        val draft = RightsDraft().applyPreset(preset(PRESET_ALL), noScreen)
        assertEquals(ACCESS_OFF, draft.rights.screenView)
        assertEquals(ACCESS_OFF, draft.rights.screenControl)
        assertEquals(PRESET_FILES_EXEC, draft.matchingPreset(presets)?.key)
        val selection = draft.selection(presets, 1_000L)
        assertEquals(PRESET_ALL, selection.preset)
        assertEquals(setOf("screenView", "screenControl"), selection.rights!!.keys)
        assertEquals("off", selection.rights!!["screenView"]!!.jsonPrimitive.content)
        assertEquals(PRESET_ALL, presetSelection(PRESET_ALL, presets, noScreen)!!.preset)
        assertNull(presetSelection(null, presets, noScreen))
        assertNull(presetSelection("gibt-es-nicht", presets, noScreen))
    }

    @Test
    fun abgleichTaskOfferedRightsFollowTheProviderAndItsCapabilityRows() {
        val rows = listOf(CapabilityRow(CAP_COMMANDS), CapabilityRow(CAP_TERMINAL), CapabilityRow(CAP_SCREEN_VIEW))
        val offered = OfferedRights.from(AccessProviderInfo(capabilities = rows), ExecProviderInfo(available = true))
        assertTrue(offered.commands)
        assertTrue(offered.terminal)
        assertTrue(offered.screenView)
        assertFalse(offered.screenControl)
        assertNull(offered.execNote)
        val down = OfferedRights.from(AccessProviderInfo(capabilities = rows), ExecProviderInfo(available = false, detail = "kein Zwischenprozess"))
        assertFalse(down.commands)
        assertFalse(down.terminal)
        assertEquals("kein Zwischenprozess", down.execNote)
        // A bridge before the capability rows: commands and terminal only.
        val older = OfferedRights.from(null, ExecProviderInfo(available = true))
        assertTrue(older.commands)
        assertFalse(older.screenView)
        val clamped = down.clamp(Rights(commands = ACCESS_ALLOW, terminal = ACCESS_ASK, account = "remote"))
        assertEquals(ACCESS_OFF, clamped.commands)
        assertEquals(ACCESS_OFF, clamped.terminal)
        assertEquals("own", clamped.account)
    }

    @Test
    fun abgleichTaskControllingTheDesktopNeedsWatchingAndWatchingOffEndsControlling() {
        val control = RightsDraft().setDecision(RIGHTS_FIELD_SCREEN_CONTROL, ACCESS_ALLOW)
        assertEquals(ACCESS_ALLOW, control.rights.screenView)
        val off = control.setDecision(RIGHTS_FIELD_SCREEN_VIEW, ACCESS_OFF)
        assertEquals(ACCESS_OFF, off.rights.screenControl)
    }

    @Test
    fun abgleichTaskWithoutAPresetEveryKnownFieldIsSentAndTheEndIsResolved() {
        val draft = RightsDraft()
            .setFolders(FOLDERS_SELECTED)
            .togglePath("/data/a")
            .togglePath("/data/b")
            .togglePath("/data/a")
            .setWrite(true)
            .setExpiry("1h")
        assertEquals(listOf("/data/b"), draft.rights.folders.paths)
        assertNull(draft.presetKey)
        val selection = draft.selection(presets, 1_000L)
        assertNull(selection.preset)
        val rights = selection.rights!!
        assertEquals("selected", rights["folders"]!!.jsonObject["kind"]!!.jsonPrimitive.content)
        assertEquals(listOf("/data/b"), rights["folders"]!!.jsonObject["paths"]!!.jsonArray.map { it.jsonPrimitive.content })
        assertEquals(4_600L, rights["validUntil"]!!.jsonPrimitive.content.toLong())
        assertEquals("own", rights["account"]!!.jsonPrimitive.content)
        assertEquals(4_600L, draft.effective(1_000L).validUntil)
        assertNull(RightsDraft().effective(1_000L).validUntil)
    }

    @Test
    fun abgleichTaskADraftThatStartsFromAnOpenConfirmationKeepsItsEndUntilChanged() {
        val seed = Rights(folders = RightsFolders(FOLDERS_ALL), write = true, validUntil = 9_999L)
        val draft = RightsDraft.fromSeed(seed, allFields, everything)
        assertEquals(EXPIRY_KEEP, draft.expiry)
        assertEquals(9_999L, draft.effective(1L).validUntil)
        assertEquals(1L + 86_400L, draft.setExpiry("1d").effective(1L).validUntil)
        assertNull(draft.setExpiry(EXPIRY_NONE).effective(1L).validUntil)
    }

    private val allFields = app.smartexplorer.android.api.RIGHTS_FIELDS.toSet()

    @Test
    fun abgleichTaskTheNextOpenConfirmationShowsUnlessItWasPutOff() {
        val first = pending(device)
        val second = pending(room, "Team")
        assertEquals(first, nextPendingSheet(listOf(first, second), emptySet()))
        assertEquals(second, nextPendingSheet(listOf(first, second), setOf(device.key)))
        assertNull(nextPendingSheet(listOf(first, second), setOf(device.key, room.key)))
        assertNull(nextPendingSheet(emptyList(), emptySet()))
        val sheet = resolveRightsSheet(null, listOf(first), emptySet())!!
        assertTrue(sheet.pending)
        assertEquals(device, sheet.target)
        assertEquals("request", sheet.origin)
        assertEquals(RightsDraft().known, sheet.known)
    }

    @Test
    fun abgleichTaskAMenuRequestWinsOverTheWaitingOnesAndOpensAWaitingTargetAsWaiting() {
        val explicit = RightsRequest(room, "Team", null, null, emptySet(), pending = false)
        assertEquals(explicit, resolveRightsSheet(explicit, listOf(pending(device)), emptySet()))
        val status = ShareStatus(pendingRights = listOf(pending(device, rights = Rights(write = true))))
        val opened = requestOfExisting(device, "Laptop", status, null)
        assertTrue(opened.pending)
        assertTrue(opened.seed!!.write)
    }

    @Test
    fun abgleichTaskRightsOfAnEarlierConnectionStartFromTheStatusAndLeaveTheFoldersOpen() {
        val grant = ShareWriteGrant("d1", "Laptop", "k1", "n1", "fp", "Accepted", write = true, active = true, canSetWrite = true)
        val status = ShareStatus(
            devices = listOf(ShareDevice(contactId = "c1", deviceId = "d1", publicKey = "k1", write = false)),
            writeGrants = listOf(grant),
        )
        val request = requestOfExisting(device, "Laptop", status, null)
        assertFalse(request.pending)
        assertTrue(request.seed!!.write)
        assertTrue(RIGHTS_FIELD_WRITE in request.known)
        assertFalse(RIGHTS_FIELD_FOLDERS in request.known)
        assertFalse(RIGHTS_FIELD_COMMANDS in request.known)
        val draft = RightsDraft.fromSeed(request.seed, request.known, everything)
        assertFalse(draft.knows(RIGHTS_FIELD_FOLDERS))
        assertNull(draft.matchingPreset(presets))
        // Confirming only what was looked at: the unknown folders and decisions are not sent.
        val sent = draft.setWrite(false).selection(presets, 1L).rights!!
        assertTrue("write" in sent)
        assertFalse("folders" in sent)
        assertFalse("commands" in sent)
        assertTrue("account" in sent)
        // A preset knows every field again.
        val withPreset = draft.applyPreset(preset(PRESET_READ_ONLY), everything)
        assertTrue(withPreset.knows(RIGHTS_FIELD_FOLDERS))
        assertEquals(PRESET_READ_ONLY, withPreset.matchingPreset(presets)?.key)
        // A room starts from its members' write right.
        val roomStatus = ShareStatus(rooms = listOf(app.smartexplorer.android.api.ShareRoom("p1", policy = app.smartexplorer.android.api.ShareRoomPolicy(true, false))))
        val roomRequest = requestOfExisting(room, "Team", roomStatus, null)
        assertTrue(roomRequest.seed!!.write)
        assertTrue(roomRequest.target.isRoom)
        assertEquals("p1", roomRequest.folderScope)
    }

    @Test
    fun abgleichTaskConfirmCallsConfirmRightsAndCancelCallsCancelRightsOnlyForAWaitingConnection() {
        val waiting = requestOfPending(pending(device))
        val draft = RightsDraft.fromSeed(waiting.seed, waiting.known, everything).applyPreset(preset(PRESET_READ_WRITE), everything)
        val confirm = confirmCall(waiting, draft, presets, 1L)
        assertEquals(device, confirm.target)
        assertEquals(PRESET_READ_WRITE, confirm.selection.preset)
        assertEquals(RightsCall.Cancel(device), cancelCall(waiting))
        val settings = waiting.copy(pending = false)
        assertEquals(RightsCall.CloseOnly, cancelCall(settings))
        assertEquals(RightsCall.Confirm(device, confirm.selection), confirmCall(settings, draft, presets, 1L))
    }

    @Test
    fun abgleichTaskDeviceRowsFindTheirOpenConfirmationByTheExactPins() {
        val waiting = listOf(pending(device), pending(room, "Team"))
        val row = ShareDevice(contactId = "c", deviceId = "d1", publicKey = "k1")
        assertNotNull(pendingFor(rightsTargetOf(row), waiting))
        // The same device id with another key is another identity.
        assertNull(pendingFor(rightsTargetOf(ShareDevice(contactId = "c", deviceId = "d1", publicKey = "other")), waiting))
        assertNull(rightsTargetOf(ShareDevice(contactId = "c")))
        assertNotNull(pendingFor(RightsTarget.room("p1"), waiting))
        assertNull(pendingFor(null, waiting))
    }

    @Test
    fun abgleichTaskSummariesNameWhatIsGiven() {
        assertEquals("Keine Rechte", rightsSummary(Rights()))
        assertEquals("Dateien nur lesen (alle Ordner)", rightsSummary(preset(PRESET_READ_ONLY).rights))
        assertEquals("Dateien lesen und schreiben (alle Ordner)", rightsSummary(preset(PRESET_READ_WRITE).rights))
        assertEquals("Dateien lesen und schreiben (alle Ordner) · Befehle · Terminal", rightsSummary(preset(PRESET_FILES_EXEC).rights))
        assertEquals(
            "Dateien lesen und schreiben (alle Ordner) · Befehle · Terminal · Desktop sehen und steuern",
            rightsSummary(preset(PRESET_ALL).rights),
        )
        assertEquals(
            "Dateien nur lesen (2 Ordner) · Befehle (fragt) · Desktop sehen (fragt)",
            rightsSummary(Rights(folders = RightsFolders(FOLDERS_SELECTED, listOf("/a", "/b")), commands = ACCESS_ASK, screenView = ACCESS_ASK)),
        )
        assertEquals("Ohne Rechte bestätigen", confirmButtonLabel(Rights()))
        assertEquals("Bestätigen", confirmButtonLabel(Rights(commands = ACCESS_ASK)))
        assertTrue(grantsAnything(Rights(write = true)))
        assertFalse(grantsAnything(Rights()))
    }

    @Test
    fun abgleichTaskTheTextsDescribeTheActualBehaviourOfAcceptingAndAutoAccept() {
        val code = preset(PRESET_READ_ONLY).rights
        // Accepting: a chosen preset wins, else the Direct code's rights, else the device waits (F1, F5).
        assertTrue(acceptRightsText(preset(PRESET_READ_WRITE), code).startsWith("Das Gerät erhält sofort: Dateien lesen und schreiben"))
        assertTrue(acceptRightsText(null, code).contains("Direct-Codes: Dateien nur lesen"))
        assertTrue(acceptRightsText(null, null).contains("noch keine Rechte"))
        assertTrue(acceptRightsText(null, Rights()).contains("keine Rechte"))
        assertTrue(acceptRightsText(preset(PRESET_NONE), null).contains("ohne Rechte"))
        // The line of the Direct code and the hint of the request policy say the same.
        assertTrue(codeRightsText(null).contains("wartet ohne Rechte"))
        assertTrue(codeRightsText(Rights()).contains("erhalten keine Rechte"))
        assertTrue(codeRightsText(code).contains("Dateien nur lesen"))
        assertEquals("Neue Zugriffsanfragen warten auf deine Zustimmung.", requestPolicyHint("Ask", code))
        assertTrue(requestPolicyHint("AutoAccept", null).contains("wartet danach ohne Rechte"))
        assertTrue(requestPolicyHint("AutoAccept", code).contains("erhält sofort: Dateien nur lesen"))
        assertTrue(requestPolicyHint("AutoAccept", Rights()).contains("keine Rechte"))
        assertEquals("Anfrageeinstellung noch nicht geladen.", requestPolicyHint(null, null))
        assertEquals("Rechte für Laptop", rightsTitle(requestOfPending(pending(device))))
        assertEquals("Rechte für Raum", rightsTitle(requestOfPending(pending(room, name = ""))))
        assertTrue(rightsIntro(requestOfPending(pending(device))).contains("Bis zur Bestätigung hat das Gerät keine Rechte"))
        assertTrue(rightsIntro(requestOfPending(pending(room, "Team"))).contains("gibt der Raum nichts frei"))
        assertEquals("Rückfreigabe geöffnet", originText("share_back"))
    }
}
