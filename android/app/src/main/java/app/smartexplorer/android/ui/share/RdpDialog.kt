package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.RdpApi
import app.smartexplorer.android.api.RdpClients
import app.smartexplorer.android.api.RdpHost
import app.smartexplorer.android.api.RdpService
import app.smartexplorer.android.api.TcpApi
import app.smartexplorer.android.api.TcpTarget
import app.smartexplorer.android.core.CoreException
import kotlin.coroutines.cancellation.CancellationException

/** Genuine host/client inventory for the same pinned Direct/room selectors as TCP. */
@Composable
internal fun RdpDialog(selector: ShareDialog.Rdp, onDismiss: () -> Unit) {
    var targets by remember(selector) { mutableStateOf<List<TcpTarget>>(emptyList()) }
    var selectedKey by remember(selector) { mutableStateOf("") }
    var localHost by remember(selector) { mutableStateOf<RdpHost?>(null) }
    var clients by remember(selector) { mutableStateOf<RdpClients?>(null) }
    var host by remember(selector) { mutableStateOf<RdpHost?>(null) }
    var loading by remember(selector) { mutableStateOf(false) }
    var probing by remember(selector) { mutableStateOf(false) }
    var error by remember(selector) { mutableStateOf<String?>(null) }
    var reload by remember(selector) { mutableIntStateOf(0) }
    var probeRevision by remember(selector) { mutableIntStateOf(0) }
    var probeGeneration by remember(selector) { mutableIntStateOf(0) }
    LaunchedEffect(selector, reload) {
        loading = true
        try {
            localHost = RdpApi.localHost()
            clients = RdpApi.clients()
            targets = TcpApi.devices().filter { target ->
                (selector.contactId != null && target.relation == "direct" && target.relationId == selector.contactId) ||
                    (selector.deviceId != null && selector.publicKey != null && target.deviceId == selector.deviceId && target.publicKey == selector.publicKey)
            }
            if (targets.none { it.key == selectedKey && it.canStart }) selectedKey = targets.firstOrNull { it.canStart }?.key.orEmpty()
            host = null; error = null
        } catch (cancel: CancellationException) { throw cancel } catch (failure: CoreException) { targets = emptyList(); host = null; error = failure.message ?: failure.kind } finally { loading = false }
    }
    val selected = targets.firstOrNull { it.key == selectedKey && it.canStart }
    LaunchedEffect(selectedKey, probeRevision) {
        val generation = ++probeGeneration
        host = null
        if (probeRevision == 0 || selected == null) { probing = false; return@LaunchedEffect }
        probing = true
        try {
            val report = RdpApi.probe(selected)
            if (generation == probeGeneration) { host = report; error = null }
        } catch (cancel: CancellationException) { throw cancel } catch (failure: CoreException) { if (generation == probeGeneration) error = failure.message ?: failure.kind } finally { if (generation == probeGeneration) probing = false }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Remote Desktop: ${selector.name.ifBlank { "Gerät" }}") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Dieses Telefon", style = MaterialTheme.typography.titleSmall)
                localHost?.unavailableReason?.let { Text(it) }
                clients?.unavailableReason?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                clients?.clients.orEmpty().forEach { client ->
                    Text("${client.id}: ${if (client.available) "verfügbar" else "nicht verfügbar"}")
                    Text(client.detail)
                }
                if (loading) Text("Geräteidentität und lokale RDP-Fähigkeit werden geladen …")
                Text("Beziehung zum Zielgerät", style = MaterialTheme.typography.titleSmall)
                targets.filter { it.canStart }.forEach { target ->
                    FilterChip(selected = selectedKey == target.key, enabled = !loading && !probing,
                        onClick = { selectedKey = target.key; host = null; probeRevision = 0 },
                        label = { Text("${target.relationLabel} / ${target.name}") })
                }
                if (selected == null && !loading) Text("Keine gespeicherte ausgehende Beziehung zu dieser bestätigten Geräteidentität.")
                selected?.let { SelectionContainer { Text("Gerät: ${it.deviceId}\nVerbindungsidentität: ${it.nodeId}\nBeziehung: ${it.relationId}") } }
                TextButton(enabled = selected != null && !loading && !probing, onClick = { probeRevision++ }) {
                    Text(if (probing) "Authentifizierte RDP-Abfrage läuft …" else "RDP-Fähigkeit des Zielgeräts abfragen")
                }
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                host?.let { report ->
                    Text("Authentifizierter Hostbestand: ${report.platform}", style = MaterialTheme.typography.titleSmall)
                    report.unavailableReason?.let { Text(it) }
                    report.diagnostics.forEach { Text(it) }
                    report.services.forEach { ServiceDetails(it) }
                }
                Text("Für einen RDP-Start ist ein belegter Client-/Zertifikatadapter erforderlich; der tatsächliche Fähigkeitsgrund steht oben. Dieser Dialog öffnet keinen TCP-Port und startet keinen App-Link. Der gemeinsame Bildschirm und die vorhandene TCP-Bedienung bleiben separat erreichbar.")
            }
        },
        confirmButton = { TextButton(enabled = !loading && !probing, onClick = { reload++ }) { Text("Geräte- und Clientbestand aktualisieren") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Schließen") } },
    )
}

@Composable
private fun ServiceDetails(service: RdpService) {
    var evidence by remember(service.id) { mutableStateOf(false) }
    Text("${service.id} · ${serviceKind(service.kind)}", style = MaterialTheme.typography.titleSmall)
    Text(sessionEffect(service.sessionMode))
    Text("Version: ${service.version ?: "nicht ermittelt"} · Port: ${service.port ?: "nicht belegt"} · ${if (service.running) "Dienst aktiv" else "Dienst inaktiv"}")
    Text("TLS: ${fact(service.tls)} · NLA: ${fact(service.nla)}")
    service.unavailableReason?.let { Text(it, color = MaterialTheme.colorScheme.error) }
    Text("SHA-256-Zertifikatfingerprint aus dem Hostbestand:")
    SelectionContainer { Text(service.certificateSha256 ?: "Nicht belegt; kein erfundener Fingerprint.") }
    service.certificateSource?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
    Text("Veröffentlichte RemoteApps", style = MaterialTheme.typography.titleSmall)
    if (service.remoteApps.isEmpty()) Text("Kein belegtes Windows-Server/RDS-RemoteAppangebot.")
    service.remoteApps.forEach { app -> Text("${app.displayName} · Alias ${app.alias} · Collection ${app.collection}") }
    TextButton(onClick = { evidence = !evidence }) { Text(if (evidence) "Hostbelege schließen" else "Hostbelege anzeigen") }
    if (evidence) service.evidence.forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
}
private fun fact(value: Boolean?): String = when (value) { true -> "aktiv / belegt"; false -> "aus"; null -> "nicht ermittelt" }
private fun serviceKind(kind: String): String = when (kind) {
    "windows_workstation" -> "Windows Pro/Enterprise"
    "windows_server" -> "Windows Server / RDS"
    "gnome" -> "GNOME Remote Desktop"
    "krdp" -> "KRdp"
    "xrdp" -> "xrdp"
    else -> "Nicht unterstützter Hosttyp dieser App-Version"
}
private fun sessionEffect(mode: String): String = when (mode) {
    "windows_console_locked" -> "Windows sperrt den lokalen Bildschirm; der angemeldete Benutzer kann getrennt werden."
    "windows_server_session" -> "Windows Server verbindet eine RDS-Sitzung; die Serverrichtlinie bestimmt eine Wiederverbindung bestehender Sitzungen."
    "shared_console" -> "Der sichtbare lokale Desktop wird geteilt."
    "exclusive_console" -> "KRdp nutzt einen virtuellen Bildschirm der bestehenden Sitzung, fordert lokal den Greeter an und sperrt die Sitzung beim letzten RDP-Verbindungsende."
    "additional_console_display" -> "KRdp erstellt einen zusätzlichen virtuellen Bildschirm in der bestehenden Sitzung; keine separate Anmeldung."
    "headless_user_session" -> "GNOME verwendet eine bildschirmlose Benutzersitzung."
    "remote_login" -> "GNOME bietet Remote Login; bestehende Sitzungen unterliegen der GDM-Richtlinie."
    "separate_xorg" -> "xrdp verbindet eine separate Xorg-Sitzung oder eine vorhandene xrdp-Sitzung wieder."
    "separate_xvnc" -> "xrdp verbindet eine separate Xvnc-Sitzung oder eine vorhandene xrdp-Sitzung wieder."
    else -> "Die konkrete Sitzungswirkung ist nicht belegt; der native Adapter weist einen Start ab."
}
