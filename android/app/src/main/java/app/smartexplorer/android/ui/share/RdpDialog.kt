package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.DeviceFeed
import app.smartexplorer.android.api.FamilyApi
import app.smartexplorer.android.api.RdpApi
import app.smartexplorer.android.api.RdpClients
import app.smartexplorer.android.api.TcpApi
import app.smartexplorer.android.api.TcpTarget
import app.smartexplorer.android.core.CoreException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/**
 * Remote Desktop of one device (design 2.4, 2.9). Android has no RDP program, so the dialog
 * shows why and the device's Remote Desktop state from the feed; „Bildschirm ansehen/steuern“
 * stays the way to this device's screen. With an RDP program one tap opens the device.
 */
@Composable
internal fun RdpDialog(selector: ShareDialog.Rdp, onDismiss: () -> Unit) {
    var clients by remember(selector) { mutableStateOf<RdpClients?>(null) }
    var target by remember(selector) { mutableStateOf<TcpTarget?>(null) }
    var feed by remember(selector) { mutableStateOf<DeviceFeed?>(null) }
    var localDiagnostics by remember(selector) { mutableStateOf<List<String>>(emptyList()) }
    var busy by remember(selector) { mutableStateOf(false) }
    var info by remember(selector) { mutableStateOf<String?>(null) }
    var error by remember(selector) { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(selector) {
        // A remote device's lines come from its own device feed (E54); this phone's probe is read only without a remote identity.
        if (rdpSelectsForeignDevice(selector.contactId, selector.deviceId, selector.publicKey)) return@LaunchedEffect
        localDiagnostics = try { RdpApi.localHost().diagnostics } catch (cancel: CancellationException) { throw cancel
        } catch (failure: CoreException) { listOf("Diagnose nicht lesbar: ${failure.message ?: failure.kind}") }
    }
    LaunchedEffect(selector) {
        try {
            clients = RdpApi.clients()
            val targets = TcpApi.devices().filter { candidate ->
                candidate.canStart && ((selector.contactId != null && candidate.relation == "direct" && candidate.relationId == selector.contactId) ||
                    (selector.deviceId != null && selector.publicKey != null && candidate.deviceId == selector.deviceId && candidate.publicKey == selector.publicKey))
            }
            target = targets.firstOrNull { it.present } ?: targets.firstOrNull()
            target?.let { feed = FamilyApi.feed(it.deviceId, it.publicKey) }
        } catch (cancel: CancellationException) { throw cancel
        } catch (failure: CoreException) { error = failure.message ?: failure.kind }
    }
    val usable = clients?.clients.orEmpty().any { it.available }
    val rdp = feed?.rdp
    val foreign = rdpSelectsForeignDevice(selector.contactId, selector.deviceId, selector.publicKey)
    val diagnosticLines = rdpDiagnosticSource(foreign, rdp?.diagnostics.orEmpty(), localDiagnostics)
    fun open(remoteApp: String?) {
        val device = target ?: return
        if (busy) return
        busy = true; error = null; info = null
        scope.launch {
            try { RdpApi.open(device.deviceId, device.publicKey, remoteApp); info = "Remote Desktop läuft." }
            catch (cancel: CancellationException) { throw cancel }
            catch (failure: CoreException) { error = failure.message ?: failure.kind }
            finally { busy = false }
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Remote Desktop: ${selector.name.ifBlank { "Gerät" }}") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (clients == null && error == null) Text("RDP-Programme dieses Telefons werden gelesen …")
                if (clients != null && !usable) {
                    Text("Auf diesem Telefon gibt es kein RDP-Programm, das se sicher starten kann; „Bildschirm ansehen“ und „Bildschirm steuern“ zeigen den Bildschirm dieses Geräts.")
                    clients?.unavailableReason?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                }
                Text("Remote Desktop des Geräts", style = MaterialTheme.typography.titleSmall)
                when {
                    target == null && clients != null -> Text("Gerät ist nicht erreichbar")
                    rdp == null -> Text(feed?.unavailable ?: "Der Remote-Desktop-Stand erscheint, sobald das Gerät dir „Bildschirm steuern“ erlaubt.")
                    rdp.startable -> Text("Bereit")
                    else -> Text(rdp.setupText)
                }
                if (diagnosticLines.isNotEmpty()) {
                    Text(if (foreign) "Diagnose des Remote-Desktop-Hosts dieses Geräts" else "Diagnose dieses Telefons", style = MaterialTheme.typography.titleSmall)
                    SelectionContainer {
                        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            diagnosticLines.forEach { line -> Text(line, style = MaterialTheme.typography.bodySmall) }
                        }
                    }
                }
                if (usable && target != null && (rdp == null || rdp.startable || rdp.setupPossible)) {
                    TextButton(enabled = !busy, onClick = { open(null) }) { Text("Remote Desktop") }
                    rdp?.takeIf { it.startable }?.remoteApps.orEmpty().forEach { app ->
                        TextButton(enabled = !busy, onClick = { open(app.alias) }) { Text("${app.displayName} (RemoteApp)") }
                    }
                }
                if (busy) Text("Remote Desktop wird geöffnet …")
                info?.let { Text(it) }
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Schließen") } },
    )
}

/**
 * The diagnostic lines to show, in the order given. Blank lines are dropped; every other line is kept
 * literally (no trimming, cutting or merging), so each line reads and copies as it was reported.
 */
internal fun rdpDiagnosticLines(diagnostics: List<String>): List<String> = diagnostics.filter { it.isNotBlank() }

/**
 * Whether the selector names a remote device: by its contact, or by its device identity together with its pinned key
 * (E54). Any other selector is this phone.
 */
internal fun rdpSelectsForeignDevice(contactId: String?, deviceId: String?, publicKey: String?): Boolean =
    contactId != null || (deviceId != null && publicKey != null)

/**
 * The lines to show (E54): a remote device's lines are the lines of its own device feed, never this phone's; this
 * phone's lines are its local probe lines.
 */
internal fun rdpDiagnosticSource(foreign: Boolean, feedLines: List<String>, localLines: List<String>): List<String> =
    rdpDiagnosticLines(if (foreign) feedLines else localLines)
