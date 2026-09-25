package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.more.ToggleSetting

/** Callbacks of the "Dieses Gerät" card. */
internal class DeviceCardActions(
    val rename: () -> Unit,
    val setOnline: (Boolean) -> Unit,
    val reconnect: () -> Unit,
    val makeDiscoverable: () -> Unit,
    val stopDiscoverable: (offerId: String) -> Unit,
    val copyDirectCode: (String) -> Unit,
)

/**
 * Card "Dieses Gerät" (spec F18): name ✎, Online/Offline/Relay (or "Nur LAN" without server),
 * online switch, [Erneut verbinden], [Suchbar machen] or the running offer with a countdown, and
 * the Direct code for "Direct-Code hinzufügen" on other devices.
 */
@Composable
internal fun ThisDeviceCard(status: ShareStatus, nowMs: Long, actions: DeviceCardActions, modifier: Modifier = Modifier) {
    val identity = status.identity
    Card(modifier.fillMaxWidth()) {
        Column(Modifier.padding(vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(Modifier.padding(start = 16.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                SeIcon(R.drawable.ic_device, contentDescription = null)
                Text(
                    identity.deviceName.ifBlank { "Dieses Gerät" },
                    style = MaterialTheme.typography.titleLarge,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f).padding(start = 12.dp),
                )
                IconButton(onClick = actions.rename) { SeIcon(R.drawable.ic_rename, contentDescription = "Namen ändern") }
            }
            val inset = Modifier.padding(horizontal = 16.dp)
            Text(onlineText(status), style = MaterialTheme.typography.titleSmall, color = onlineColor(status), modifier = inset)
            Text(
                status.server?.let { "Share-Server: $it" } ?: "Kein Share-Server – nur LAN (Einstellungen → Share-Server)",
                style = MaterialTheme.typography.bodySmall,
                modifier = inset,
            )
            if (status.lanPresence.isNotBlank()) {
                Text("LAN: ${status.lanPresence}", style = MaterialTheme.typography.bodySmall, modifier = inset)
            }
            status.lastError?.takeIf { it.isNotBlank() }?.let {
                Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, modifier = inset)
            }
            ToggleSetting("Online", status.running, actions.setOnline)
            if (status.running && status.server != null && !status.connected) {
                OutlinedButton(onClick = actions.reconnect, modifier = inset) { Text("Erneut verbinden") }
            }
            val offer = status.discovery.offer
            if (offer != null) {
                val left = ((offer.untilMs - nowMs) / 1000).coerceAtLeast(0)
                Text(
                    "Suchbar als „${offer.alias}“ – noch %d:%02d".format(left / 60, left % 60),
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = inset,
                )
                OutlinedButton(onClick = { actions.stopDiscoverable(offer.offerId) }, modifier = inset) { Text("Suchbar beenden") }
            } else {
                Button(onClick = actions.makeDiscoverable, modifier = inset) { Text("Suchbar machen") }
            }
            if (identity.directCode.isNotBlank()) {
                TextButton(onClick = { actions.copyDirectCode(identity.directCode) }, modifier = Modifier.padding(start = 4.dp)) {
                    Text("Direct-Code kopieren")
                }
            }
            if (identity.fingerprint.isNotBlank()) {
                Text(
                    "Fingerabdruck: ${identity.fingerprint}",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = inset,
                )
            }
        }
    }
}

/** Offline / Nur LAN / Online / Online über Relay (spec F18 "Status Online/Offline/Relay"). */
internal fun onlineText(status: ShareStatus): String = when {
    !status.running -> "Offline"
    status.server == null -> "Online – nur LAN"
    status.connected && status.relayUrl != null -> "Online über Relay"
    status.connected -> "Online"
    else -> "Offline – Server nicht erreichbar"
}

@Composable
private fun onlineColor(status: ShareStatus) =
    if (status.running && (status.connected || status.server == null)) {
        MaterialTheme.colorScheme.primary
    } else {
        MaterialTheme.colorScheme.error
    }
