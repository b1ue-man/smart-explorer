package app.smartexplorer.android.ui.share

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import app.smartexplorer.android.R
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.system.Permissions
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import app.smartexplorer.android.api.PendingRights
import app.smartexplorer.android.api.RightsTarget
import app.smartexplorer.android.api.ShareDevice
import app.smartexplorer.android.api.ShareMember
import app.smartexplorer.android.api.ScreenDevice
import app.smartexplorer.android.api.ShareRoom
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SectionHeader

/** A device reachable over Share: a Direct device or a room member. */
internal data class PeerTarget(val name: String, val location: String)

/**
 * What the rows need for the screen entries (SCR-M9): the devices the core knows (`screen.devices`,
 * with the result of the last probe) and what the entries do.
 */
internal class ScreenLinks(
    val devices: List<ScreenDevice>,
    /** Opens the full view of a device; the flag starts with control. */
    val open: (ScreenDevice, Boolean) -> Unit,
    /** Opens the group view of a room (its profile ID). */
    val openRoom: (String) -> Unit,
) {
    fun find(relation: String, relationId: String, deviceId: String?): ScreenDevice? =
        devices.firstOrNull { it.relation == relation && it.relationId == relationId && (deviceId == null || it.deviceId == deviceId) }
}

/** Callbacks of the device and room rows. */
internal class PeerActions(
    val open: (location: String) -> Unit,
    val sendFiles: (PeerTarget) -> Unit,
    val exec: (PeerTarget) -> Unit,
    val requestAccess: (ShareDevice) -> Unit,
    val removeDevice: (ShareDevice) -> Unit,
    val showRoomCode: (ShareRoom) -> Unit,
    val leaveRoom: (ShareRoom) -> Unit,
    val removeRoom: (ShareRoom) -> Unit,
    val shareBack: (ShareDevice) -> Unit,
    val roomPolicy: (ShareRoom) -> Unit,
    val roomMember: (ShareRoom, ShareMember, String) -> Unit,
    /** „Verbindung…“: connect/open automatically, reset trust. */
    val connection: (ShareDevice) -> Unit,
    val renameRoom: (ShareRoom) -> Unit,
    val rejoinRoom: (ShareRoom) -> Unit,
    val resetMemberTrust: (ShareRoom, ShareMember) -> Unit,
    /** „Rechte einstellen…“ / „Rechte festlegen…“: the compact rights menu of a Direct device. */
    val rights: (ShareDevice) -> Unit,
    /** The same for a room. */
    val roomRights: (ShareRoom) -> Unit,
    val screens: ScreenLinks,
)

/** „Bildschirm…“ and, unless the device does not allow control, „Bildschirm steuern“ for [screen]. */
@Composable
private fun screenEntries(screen: ScreenDevice?, screens: ScreenLinks): List<Pair<String, () -> Unit>> {
    if (screen == null) return emptyList()
    val open = stringResource(R.string.screen_menu_open) to { screens.open(screen, false) }
    // A device whose last probe said „control: off“ does not get the entry; an unknown one does.
    val control = stringResource(R.string.screen_menu_control) to { screens.open(screen, true) }
    return if (screen.control == "off") listOf(open) else listOf(open, control)
}

/** One rejected session the worker reports (`share.access`, R3 of plan 2026-11-06). */
@Serializable
internal data class AccessNotice(
    val direction: String,
    val deviceId: String,
    val deviceName: String? = null,
    val contactId: String? = null,
    val reason: String = "",
    val message: String = "",
    val remedy: String = "",
    val count: Long = 0,
    val retryInSecs: Long? = null,
)

/** A paired device that cannot open this phone's exports (R1/R9). */
@Serializable
internal data class AccessGap(
    val kind: String,
    val deviceId: String,
    val deviceName: String = "",
    val contactId: String? = null,
    val message: String = "",
    val action: String = "",
)

/** Which of this phone's folders one Direct grant sees (R5). */
@Serializable
internal data class DeviceFolders(
    val deviceId: String,
    val publicKey: String,
    /** `all`, `none` or `selected`. */
    val choice: String,
    val count: Int = 0,
)

@Serializable
internal data class AccessFacts(
    val deviceFolders: List<DeviceFolders> = emptyList(),
    val notices: List<AccessNotice> = emptyList(),
    val gaps: List<AccessGap> = emptyList(),
    val reconfirm: Int = 0,
    val storageAccess: Boolean? = null,
    /** Whether this phone shares any folder (Direct or in a room). */
    val sharesFolders: Boolean = false,
    val workerError: String? = null,
)

private const val ACCESS_REFRESH_MS = 15_000L

/**
 * „Zugriff“: rejected sessions in both directions with reason and way to allow them, grants
 * waiting for re-confirmation („Alle neu bestätigen“), one-way pairings („Eigene Freigaben
 * öffnen“) and a missing „Zugriff auf alle Dateien“ while this phone shares folders. Read only
 * while the app is visible; nothing is shown when everything is allowed.
 */
@Composable
internal fun AccessSection() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val owner = LocalLifecycleOwner.current
    var facts by remember { mutableStateOf<AccessFacts?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var reloads by remember { mutableIntStateOf(0) }
    LaunchedEffect(owner, reloads) {
        owner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                try {
                    facts = Core.request<AccessFacts>("share.access")
                } catch (cancel: CancellationException) {
                    throw cancel
                } catch (e: CoreException) {
                    failure = e.message ?: e.kind
                }
                delay(ACCESS_REFRESH_MS)
            }
        }
    }
    fun act(method: String, args: kotlinx.serialization.json.JsonObject) {
        scope.launch {
            failure = try {
                Core.call(method, args)
                null
            } catch (e: CoreException) {
                e.message ?: e.kind
            }
            reloads++
        }
    }
    val current = facts ?: return
    val storageMissing = current.sharesFolders && current.storageAccess == false
    val gaps = current.gaps.filter { it.kind != "rights_pending" }
    if (current.notices.isEmpty() && gaps.isEmpty() && !storageMissing && failure == null) return
    SectionHeader("Zugriff")
    if (storageMissing) {
        HintLine("„Zugriff auf alle Dateien“ fehlt: andere Geräte sehen in deinen Freigaben im gemeinsamen Speicher nur einen Teil der Dateien (oft leere Ordner).")
        TextButton(onClick = { Permissions.openAllFilesAccessSettings(context) }, modifier = Modifier.padding(horizontal = 8.dp)) {
            Text("Zugriff auf alle Dateien erlauben …")
        }
    }
    if (current.reconfirm > 0) {
        HintLine("${current.reconfirm} Gerät(e) warten auf die erneute Bestätigung (Direkt-Code erneuert oder Identität repariert) und erreichen deine Freigaben bis dahin nicht.")
        TextButton(onClick = { act("share.reconfirmAll", buildJsonObject { }) }, modifier = Modifier.padding(horizontal = 8.dp)) {
            Text("Alle neu bestätigen")
        }
    }
    gaps.filter { it.kind == "not_shared" }.forEach { gap ->
        HintLine(gap.message)
        val contact = gap.contactId
        if (contact != null) {
            TextButton(
                onClick = { act("share.setShareBack", buildJsonObject { put("contactId", contact); put("shareBack", true) }) },
                modifier = Modifier.padding(horizontal = 8.dp),
            ) { Text(gap.action.ifBlank { "Eigene Freigaben öffnen" }) }
        }
    }
    current.notices.forEach { notice ->
        val name = notice.deviceName?.takeIf { it.isNotBlank() } ?: notice.deviceId
        val title = if (notice.direction == "outgoing") "$name lehnt dieses Telefon ab" else "$name wurde hier abgelehnt"
        val retry = notice.retryInSecs?.let { " · nächster Versuch in $it s" } ?: ""
        ListItem(
            headlineContent = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            supportingContent = { Text("${notice.message}. ${notice.remedy}. (${notice.count}×$retry)") },
            leadingContent = { SeIcon(R.drawable.ic_device, contentDescription = null) },
        )
    }
    failure?.let { HintLine("Zugriffsstand nicht lesbar: $it") }
}

/** Section "Geräte": Direct devices; tap opens the device's files. */
@Composable
internal fun DevicesSection(devices: List<ShareDevice>, pending: List<PendingRights>, actions: PeerActions) {
    AccessSection()
    SectionHeader("Geräte")
    if (devices.isEmpty()) {
        HintLine("Noch keine Geräte. „Gerät verbinden“ koppelt per PIN, „Direct-Code hinzufügen“ per Code.")
        return
    }
    devices.forEach { device -> DeviceRow(device, pendingFor(rightsTargetOf(device), pending) != null, actions) }
}

@Composable
private fun DeviceRow(device: ShareDevice, rightsOpen: Boolean, actions: PeerActions) {
    val name = device.name.ifBlank { "Gerät" }
    val target = PeerTarget(name, device.location)
    val status = buildString {
        append(device.statusText.ifBlank { statusLabel(device.status) })
        if (device.lan) append(" · LAN")
        append(if (device.shareBack) " · Rückfreigabe an" else " · Keine neue Rückfreigabe")
        if (rightsOpen) append(" · Rechte offen") else device.write?.let { append(if (it) " · Schreibrecht" else " · Nur lesen") }
        if (!device.autoConnect) append(" · Verbindet nicht automatisch")
        if (device.autoOpen) append(" · Öffnet automatisch")
    }
    ListItem(
        headlineContent = { Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = {
            Column {
                Text(status, maxLines = 3, overflow = TextOverflow.Ellipsis)
                if (rightsOpen) TextButton(onClick = { actions.rights(device) }) { Text("Rechte festlegen …") }
            }
        },
        leadingContent = {
            SeIcon(
                R.drawable.ic_device,
                contentDescription = if (device.online) "online" else "offline",
                tint = if (device.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        trailingContent = {
            val screen = screenEntries(actions.screens.find("direct", device.contactId, null), actions.screens)
            RowMenu(
                listOf(
                    "Dateien öffnen" to { actions.open(device.location) },
                    "Datei senden" to { actions.sendFiles(target) },
                    "Befehl ausführen" to { actions.exec(target) },
                ) + screen + listOf(
                    (if (rightsOpen) "Rechte festlegen…" else "Rechte einstellen…") to { actions.rights(device) },
                    "Zugriff anfragen" to { actions.requestAccess(device) },
                    "Eigene Freigaben…" to { actions.shareBack(device) },
                    "Geräteeinstellungen…" to { actions.connection(device) },
                    "Entfernen" to { actions.removeDevice(device) },
                ),
            )
        },
        modifier = Modifier.clickable(enabled = device.location.isNotBlank()) { actions.open(device.location) },
    )
}

/** Section "Räume": name, members; tap opens the room, the arrow shows its members. */
@Composable
internal fun RoomsSection(rooms: List<ShareRoom>, pending: List<PendingRights>, actions: PeerActions) {
    SectionHeader("Räume")
    if (rooms.isEmpty()) {
        HintLine("Noch keine Räume. Ein Raum verbindet mehrere Geräte über einen gemeinsamen Code.")
        return
    }
    rooms.forEach { room -> RoomBlock(room, pendingFor(RightsTarget.room(room.profileId), pending) != null, actions) }
}

@Composable
private fun RoomBlock(room: ShareRoom, rightsOpen: Boolean, actions: PeerActions) {
    var expanded by rememberSaveable(room.profileId) { mutableStateOf(false) }
    val name = room.name.ifBlank { "Raum" }
    val members = room.members.size
    val location = room.location
    ListItem(
        headlineContent = { Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = {
            val state = if (room.autoJoin) statusLabel(room.status) else "Verlassen"
            val pending = room.members.count { it.admission == "Pending" }
            val rights = room.policy?.let { if (it.membersMayWrite) "Mitglieder dürfen schreiben" else "Mitglieder nur lesend" } ?: "Rechte unbekannt"
            Column {
                Text("$state · $members ${if (members == 1) "Mitglied" else "Mitglieder"}" +
                    (if (pending > 0) " · $pending warten auf Zustimmung" else "") + "\n" + if (rightsOpen) "Rechte offen" else rights)
                if (rightsOpen) TextButton(onClick = { actions.roomRights(room) }) { Text("Rechte festlegen …") }
            }
        },
        leadingContent = { SeIcon(R.drawable.ic_room, contentDescription = null) },
        trailingContent = {
            Row {
                if (members > 0) {
                    IconButton(onClick = { expanded = !expanded }) {
                        SeIcon(
                            if (expanded) R.drawable.ic_expand_less else R.drawable.ic_expand_more,
                            contentDescription = if (expanded) "Mitglieder ausblenden" else "Mitglieder zeigen",
                        )
                    }
                }
                val roomScreens = stringResource(R.string.screen_menu_room)
                RowMenu(
                    buildList<Pair<String, () -> Unit>> {
                        add("Code anzeigen" to { actions.showRoomCode(room) })
                        if (room.autoJoin && members > 0) add(roomScreens to { actions.screens.openRoom(room.profileId) })
                        add((if (rightsOpen) "Rechte festlegen…" else "Rechte einstellen…") to { actions.roomRights(room) })
                        add("Rechte und Bestätigung…" to { actions.roomPolicy(room) })
                        add("Umbenennen…" to { actions.renameRoom(room) })
                        if (room.autoJoin) add("Verlassen" to { actions.leaveRoom(room) }) else add("Wieder beitreten" to { actions.rejoinRoom(room) })
                        add("Entfernen" to { actions.removeRoom(room) })
                    },
                )
            }
        },
        modifier = Modifier.clickable {
            if (location != null) {
                actions.open(location)
            } else {
                expanded = !expanded
            }
        },
    )
    if (expanded) {
        Column(Modifier.padding(start = 24.dp)) {
            room.members.forEach { member -> MemberRow(room, member, actions) }
        }
    }
}

@Composable
private fun MemberRow(room: ShareRoom, member: ShareMember, actions: PeerActions) {
    val name = member.name.ifBlank { "Gerät" }
    val target = PeerTarget(name, member.location)
    val pending = member.admission == "Pending"
    val usable = member.location.isNotBlank() && !member.blocked && !pending
    val screen = screenEntries(if (usable) actions.screens.find("room", room.profileId, member.deviceId) else null, actions.screens)
    ListItem(
        headlineContent = { Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(if (member.blocked) "Gesperrt" else if (pending) "Wartet auf deine Zustimmung" else statusLabel(member.status)) },
        leadingContent = { SeIcon(R.drawable.ic_device, contentDescription = null) },
        trailingContent = {
            RowMenu(buildList<Pair<String, () -> Unit>> {
                if (usable) {
                    add("Dateien öffnen" to { actions.open(member.location) })
                    add("Datei senden" to { actions.sendFiles(target) })
                    add("Befehl ausführen" to { actions.exec(target) })
                    addAll(screen)
                }
                when {
                    pending -> add("Zulassen…" to { actions.roomMember(room, member, "admit") })
                    member.blocked -> add("Wieder zulassen…" to { actions.roomMember(room, member, "allow") })
                    else -> add("Sperren…" to { actions.roomMember(room, member, "block") })
                }
                add("Geräteeinstellungen…" to { actions.roomMember(room, member, "settings") })
                add("Vertrauen zurücksetzen" to { actions.resetMemberTrust(room, member) })
            })
        },
        modifier = Modifier.clickable(enabled = usable) { actions.open(member.location) },
    )
}

/** ⋮ with the given entries. */
@Composable
internal fun RowMenu(entries: List<Pair<String, () -> Unit>>) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { SeIcon(R.drawable.ic_more_vert, contentDescription = "Aktionen") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            entries.forEach { (label, action) ->
                DropdownMenuItem(text = { Text(label) }, onClick = {
                    open = false
                    action()
                })
            }
        }
    }
}

/** German text for a status such as `connected`, `ConnectedRelay` or `waiting_for_access`. */
internal fun statusLabel(status: String): String {
    val key = status.lowercase().filter { it.isLetter() }
    return when {
        key.isEmpty() -> "Unbekannt"
        key == "offline" -> "Offline"
        key == "waitingforaccess" -> "Wartet auf Zugriff"
        key.startsWith("waiting") -> "Wartet"
        key == "available" -> "Erreichbar"
        key == "connecting" -> "Verbinde …"
        key == "connecteddirect" -> "Direkt verbunden"
        key == "connectedrelay" -> "Über Relay verbunden"
        key.startsWith("connected") -> "Verbunden"
        key.startsWith("failed") -> "Fehler"
        key == "identityconflict" -> "Identitätskonflikt"
        else -> status
    }
}
