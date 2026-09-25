package app.smartexplorer.android.ui.share

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.Button
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedCard
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.RemovedDevice
import app.smartexplorer.android.api.SHARE_DIRECT
import app.smartexplorer.android.api.ShareExport
import app.smartexplorer.android.api.ShareRequestInfo
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SectionHeader

/** Callbacks of the request cards. */
internal class RequestActions(
    val decide: (request: ShareRequestInfo, accept: Boolean) -> Unit,
    val retry: (ShareRequestInfo) -> Unit,
    val delete: (ShareRequestInfo) -> Unit,
)

/** Section "Anfragen" (only shown when there are any): incoming first, then outgoing. */
@Composable
internal fun RequestsSection(status: ShareStatus, actions: RequestActions) {
    SectionHeader("Anfragen")
    status.incoming.forEach { RequestCard(it, incoming = true, actions) }
    status.outgoing.forEach { RequestCard(it, incoming = false, actions) }
}

@Composable
private fun RequestCard(request: ShareRequestInfo, incoming: Boolean, actions: RequestActions) {
    OutlinedCard(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
        Column(Modifier.padding(12.dp)) {
            Text(
                (if (incoming) "Von " else "An ") + request.name.ifBlank { "unbekanntem Gerät" },
                style = MaterialTheme.typography.titleSmall,
            )
            val line = listOfNotNull(request.stateText.takeIf { it.isNotBlank() }, Format.dateTime(request.timeMs).takeIf { request.timeMs > 0 })
            if (line.isNotEmpty()) Text(line.joinToString(" · "), style = MaterialTheme.typography.bodySmall)
            request.message?.takeIf { it.isNotBlank() }?.let { Text("„$it“", style = MaterialTheme.typography.bodyMedium) }
            Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (request.canAccept) Button(onClick = { actions.decide(request, true) }) { Text("Annehmen") }
                if (request.canReject) OutlinedButton(onClick = { actions.decide(request, false) }) { Text("Ablehnen") }
                if (request.canRetry) OutlinedButton(onClick = { actions.retry(request) }) { Text("Erneut senden") }
                if (request.canDelete) TextButton(onClick = { actions.delete(request) }) { Text("Löschen") }
            }
        }
    }
}

/**
 * Section "Freigaben": local folders this device offers to Direct devices and per room, each with
 * [+ Ordner freigeben] and ⋮ Entfernen. [onAdd]/[onRemove] get the scope (`direct` or a room
 * profile id).
 */
@Composable
internal fun ExportsSection(status: ShareStatus, onAdd: (scope: String) -> Unit, onRemove: (scope: String, export: ShareExport) -> Unit) {
    SectionHeader("Freigaben")
    HintLine("Ordner, die andere Geräte auf diesem Telefon sehen dürfen.")
    ExportGroup("Für Direkt-Geräte", SHARE_DIRECT, status.exports.direct, onAdd, onRemove)
    status.rooms.forEach { room ->
        ExportGroup("Raum „${room.name.ifBlank { "Raum" }}“", room.profileId, status.exports.rooms[room.profileId].orEmpty(), onAdd, onRemove)
    }
}

@Composable
private fun ExportGroup(
    title: String,
    scope: String,
    exports: List<ShareExport>,
    onAdd: (String) -> Unit,
    onRemove: (String, ShareExport) -> Unit,
) {
    Text(title, style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(start = 16.dp, top = 12.dp))
    if (exports.isEmpty()) HintLine("Keine Ordner freigegeben.")
    exports.forEach { export ->
        ListItem(
            headlineContent = { Text(export.label.ifBlank { export.path }, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            supportingContent = { Text(export.path, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            leadingContent = { SeIcon(R.drawable.ic_folder, contentDescription = null) },
            trailingContent = { RowMenu(listOf("Entfernen" to { onRemove(scope, export) })) },
        )
    }
    TextButton(onClick = { onAdd(scope) }, modifier = Modifier.padding(start = 4.dp)) { Text("+ Ordner freigeben") }
}

/** Section "Entfernte Geräte" (only shown when there are any): [Wieder zulassen]. */
@Composable
internal fun RemovedDevicesSection(devices: List<RemovedDevice>, onReadmit: (RemovedDevice) -> Unit) {
    SectionHeader("Entfernte Geräte")
    devices.forEach { device ->
        ListItem(
            headlineContent = { Text(device.name.ifBlank { device.deviceId }, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            supportingContent = { Text("Kann sich nicht erneut koppeln, bis es wieder zugelassen wird.") },
            trailingContent = { TextButton(onClick = { onReadmit(device) }) { Text("Wieder zulassen") } },
        )
    }
}
