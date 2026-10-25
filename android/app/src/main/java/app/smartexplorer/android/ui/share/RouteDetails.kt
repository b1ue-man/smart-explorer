package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.ConnectionRoute
import app.smartexplorer.android.api.DeviceRoute
import app.smartexplorer.android.api.RouteApi
import app.smartexplorer.android.api.RouteMeasurement
import app.smartexplorer.android.api.RouteStatus
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Format
import kotlinx.coroutines.delay
import kotlin.coroutines.cancellation.CancellationException

/** Insert on the existing device page. Reads only its pinned identity's route facts. */
@Composable
internal fun RouteDetails(contactId: String? = null, deviceId: String? = null, publicKey: String? = null) {
    var snapshot by remember(contactId, deviceId, publicKey) { mutableStateOf<RouteStatus?>(null) }
    var error by remember(contactId, deviceId, publicKey) { mutableStateOf<String?>(null) }
    LaunchedEffect(contactId, deviceId, publicKey) {
        if (contactId == null && (deviceId == null || publicKey == null)) return@LaunchedEffect
        while (true) {
            try {
                snapshot = RouteApi.status(contactId, deviceId, publicKey)
                error = null
            } catch (cancel: CancellationException) { throw cancel } catch (failure: CoreException) { snapshot = null; error = failure.message ?: failure.kind } catch (failure: Exception) { snapshot = null; error = failure.message ?: "Routediagnose konnte nicht gelesen werden" }
            delay(2_000)
        }
    }
    Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text("Verbindungsweg", style = MaterialTheme.typography.titleMedium)
        if (contactId == null && (deviceId == null || publicKey == null)) {
            Text("Die Geräteidentität ist noch nicht bestätigt.")
        } else if (error != null) {
            Text("Routediagnose nicht geladen: $error", color = MaterialTheme.colorScheme.error)
        } else if (snapshot == null) {
            Text("Routediagnose wird geladen …")
        } else {
            snapshot?.let { RouteSnapshotDetails(it) }
        }
    }
}

@Composable
private fun RouteSnapshotDetails(snapshot: RouteStatus) {
    if (!snapshot.available) { Text(snapshot.reason ?: "Für diesen Worker ist keine Routediagnose verfügbar."); return }
    snapshot.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
    if (snapshot.truncated) Text("Routediagnose für die IPC-Größe gekürzt.")
    if (snapshot.peers.isEmpty() && snapshot.error == null) Text("Keine aktive Routenbeobachtung oder gespeicherte Messung für dieses Gerät.")
    snapshot.peers.forEach { peer -> PeerRouteDetails(peer, snapshot.measurementNote) }
}

@Composable
private fun PeerRouteDetails(peer: DeviceRoute, measurementNote: String?) {
    var history by remember(peer.deviceId, peer.publicKey) { mutableStateOf(false) }
    if (peer.connections.isEmpty()) Text("Keine aktive Routenbeobachtung; vorhandene Messungen sind Verlauf.")
    peer.connections.forEach { connection -> ConnectionDetails(connection) }
    if (peer.measurements.isNotEmpty()) {
        TextButton(onClick = { history = !history }) { Text("Gemessener Verlauf ${if (history) "schließen" else "anzeigen"}") }
        if (history) {
            measurementNote?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
            peer.measurements.asReversed().forEach { sample -> MeasurementDetails(sample) }
        }
    }
}

@Composable
private fun ConnectionDetails(connection: ConnectionRoute) {
    var details by remember(connection.connectionId) { mutableStateOf(false) }
    Text(connection.label, style = MaterialTheme.typography.titleSmall)
    Text(connection.reason)
    connection.remoteLimit?.let { Text(it) }
    TextButton(onClick = { details = !details }) { Text("Routendetails · ${connection.protocol} ${if (details) "schließen" else "anzeigen"}") }
    if (details) {
        Text("Netzgeneration ${connection.generation}; Revision ${connection.revision}; Pfadwechselstand ${connection.pauseEpoch}")
        connection.budget?.let { budget ->
            Text("PTO initial/bester: ${budget.initialPtoMs}/${budget.bestPtoMs} ms; Sammelfenster: ${budget.collectionMs} ms")
            Text("Antwortbudget: ${budget.responseMs} ms; Validierungsbudget: ${budget.validationMs} ms")
            budget.maxProbeAttempts?.let { Text("Maximale Probeversuche: $it; Retryintervalle: ${budget.retryIntervalsMs.joinToString(", ")} ms") }
            budget.directDeadlineMs?.let { Text("Direktfrist: $it ms ab Beginn dieser Netzgeneration") }
            budget.frozenCandidates?.let { Text("Eingefrorene Direktkandidaten: $it") }
        }
    }
}

@Composable
private fun MeasurementDetails(sample: RouteMeasurement) {
    val path = when (sample.pathClass) { "lan" -> "LAN/WLAN"; "direct" -> "Direkt"; "relay" -> "Relay"; else -> "unbekannte Pfadklasse" }
    Text(buildString {
        append("$path; Netzgeneration ${sample.generation}; TX-Frame-Nutzlast ${sample.serializedTxFramePayloadBytes} B; RX-Frame-Nutzlast ${sample.authenticatedRxFramePayloadBytes} B; Messintervall ${sample.elapsedMs} ms")
        sample.rttUs?.let { append("; RTT $it µs") }
        sample.observedAtUnixMs?.let { append("; ${Format.dateTime(it)}") }
    }, style = MaterialTheme.typography.bodySmall)
}
