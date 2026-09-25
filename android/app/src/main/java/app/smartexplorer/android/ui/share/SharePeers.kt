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
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.ShareDevice
import app.smartexplorer.android.api.ShareMember
import app.smartexplorer.android.api.ShareRoom
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SectionHeader

/** A device reachable over Share: a Direct device or a room member. */
internal data class PeerTarget(val name: String, val location: String)

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
)

/** Section "Geräte": Direct devices; tap opens the device's files. */
@Composable
internal fun DevicesSection(devices: List<ShareDevice>, actions: PeerActions) {
    SectionHeader("Geräte")
    if (devices.isEmpty()) {
        HintLine("Noch keine Geräte. „Gerät verbinden“ koppelt per PIN, „Direct-Code hinzufügen“ per Code.")
        return
    }
    devices.forEach { device -> DeviceRow(device, actions) }
}

@Composable
private fun DeviceRow(device: ShareDevice, actions: PeerActions) {
    val name = device.name.ifBlank { "Gerät" }
    val target = PeerTarget(name, device.location)
    val status = buildString {
        append(device.statusText.ifBlank { statusLabel(device.status) })
        if (device.lan) append(" · LAN")
    }
    ListItem(
        headlineContent = { Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(status, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        leadingContent = {
            SeIcon(
                R.drawable.ic_device,
                contentDescription = if (device.online) "online" else "offline",
                tint = if (device.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        trailingContent = {
            RowMenu(
                listOf(
                    "Dateien öffnen" to { actions.open(device.location) },
                    "Datei senden" to { actions.sendFiles(target) },
                    "Befehl ausführen" to { actions.exec(target) },
                    "Zugriff anfragen" to { actions.requestAccess(device) },
                    "Entfernen" to { actions.removeDevice(device) },
                ),
            )
        },
        modifier = Modifier.clickable(enabled = device.location.isNotBlank()) { actions.open(device.location) },
    )
}

/** Section "Räume": name, members; tap opens the room, the arrow shows its members. */
@Composable
internal fun RoomsSection(rooms: List<ShareRoom>, actions: PeerActions) {
    SectionHeader("Räume")
    if (rooms.isEmpty()) {
        HintLine("Noch keine Räume. Ein Raum verbindet mehrere Geräte über einen gemeinsamen Code.")
        return
    }
    rooms.forEach { room -> RoomBlock(room, actions) }
}

@Composable
private fun RoomBlock(room: ShareRoom, actions: PeerActions) {
    var expanded by rememberSaveable(room.profileId) { mutableStateOf(false) }
    val name = room.name.ifBlank { "Raum" }
    val members = room.members.size
    val location = room.location
    ListItem(
        headlineContent = { Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = {
            val state = if (room.autoJoin) statusLabel(room.status) else "Verlassen"
            Text("$state · $members ${if (members == 1) "Mitglied" else "Mitglieder"}")
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
                RowMenu(
                    buildList<Pair<String, () -> Unit>> {
                        add("Code anzeigen" to { actions.showRoomCode(room) })
                        if (room.autoJoin) add("Verlassen" to { actions.leaveRoom(room) })
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
            room.members.forEach { member -> MemberRow(member, actions) }
        }
    }
}

@Composable
private fun MemberRow(member: ShareMember, actions: PeerActions) {
    val name = member.name.ifBlank { "Gerät" }
    val target = PeerTarget(name, member.location)
    val usable = member.location.isNotBlank() && !member.blocked
    ListItem(
        headlineContent = { Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(if (member.blocked) "Gesperrt" else statusLabel(member.status)) },
        leadingContent = { SeIcon(R.drawable.ic_device, contentDescription = null) },
        trailingContent = if (usable) {
            {
                RowMenu(
                    listOf(
                        "Dateien öffnen" to { actions.open(member.location) },
                        "Datei senden" to { actions.sendFiles(target) },
                        "Befehl ausführen" to { actions.exec(target) },
                    ),
                )
            }
        } else {
            null
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
