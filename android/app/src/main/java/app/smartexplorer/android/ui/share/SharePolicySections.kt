package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedCard
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.ShareWriteGrant
import app.smartexplorer.android.api.UnconfirmedPairing
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SectionHeader

/** Local grants are not inferred from outgoing contacts or names. Every action retains full pins. */
@Composable
internal fun ContactRightsSection(grants: List<ShareWriteGrant>?, open: (ShareDialog) -> Unit) {
    SectionHeader("Zugriff auf diesem Telefon")
    HintLine("Schreibrecht gilt je bestätigter Identität und nur auf schreibbaren Freigaben. Befehle haben eine eigene Erlaubnis.")
    if (grants == null) HintLine("Kontaktrechte nicht bekannt. Status erneut laden; ältere Antworten erzeugen keine Rechteänderung.")
    else if (grants.isEmpty()) HintLine("Noch keine lokalen Kontaktfreigaben. Neue Zugriffsanfragen warten auf Zustimmung.")
    grants.orEmpty().forEach { grant ->
        OutlinedCard(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text(grant.name.ifBlank { grant.deviceId }, style = MaterialTheme.typography.titleSmall)
                Text((if (grant.active) "Aktiv" else if (grant.state == "Reconfirm") "Neu bestätigen" else "Inaktiv") +
                    " · " + if (grant.write) "Schreibrecht gespeichert" else "Nur lesen")
                Text("Fingerabdruck: ${grant.fingerprint}", style = MaterialTheme.typography.bodySmall)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(onClick = { open(ShareDialog.ContactWrite(grant)) }, enabled = grant.canSetWrite) { Text("Schreibrecht…") }
                    if (grant.active) TextButton(onClick = { open(ShareDialog.WithdrawGrant(grant)) }) { Text("Zugriff entziehen…") }
                    if (!grant.active) TextButton(onClick = { open(ShareDialog.AllowGrant(grant)) }) { Text("Wieder zulassen…") }
                }
            }
        }
    }
}

@Composable
internal fun UnconfirmedPairingsSection(pairings: List<UnconfirmedPairing>, open: (ShareDialog) -> Unit) {
    if (pairings.isEmpty()) return
    SectionHeader("Gekoppelt – Bestätigung fehlt")
    pairings.forEach { pairing ->
        OutlinedCard(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text(pairing.label)
                Text("Eine Kopplung wurde bereits installiert oder Raumdaten wurden übergeben; die Gegenseite hat dies noch nicht bestätigt.")
                if (!pairing.revocable) Text("Übergebene Raumdaten lassen sich nicht zurückholen. Bei Bedarf einen neuen Raum anlegen.")
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (pairing.revocable) TextButton(onClick = { open(ShareDialog.PairingDecision(pairing, true)) }) { Text("Widerrufen…") }
                    TextButton(onClick = { open(ShareDialog.PairingDecision(pairing, false)) }) {
                        Text(if (pairing.revocable) "Behalten…" else "Hinweis schließen…")
                    }
                }
            }
        }
    }
}
