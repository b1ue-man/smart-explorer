package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.TCP_ANY_APP_NOTICE
import app.smartexplorer.android.api.TcpApi
import app.smartexplorer.android.api.TcpServices
import app.smartexplorer.android.api.TcpTarget
import app.smartexplorer.android.api.TcpTunnel
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Format
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/**
 * TCP of one device (design 4, user rule r5): the services it shares under „Bildschirm steuern“
 * are listed by themselves and opened with one tap; a further forwarding needs no
 * acknowledgement and has no fixed lifetime. The relation is the one se uses now.
 */
@Composable
internal fun TcpDialog(selector: ShareDialog.Tcp, onDismiss: () -> Unit) {
    var targets by remember(selector) { mutableStateOf<List<TcpTarget>>(emptyList()) }
    var tunnels by remember(selector) { mutableStateOf<List<TcpTunnel>>(emptyList()) }
    var services by remember(selector) { mutableStateOf<TcpServices?>(null) }
    var host by rememberSaveable(selector) { mutableStateOf("loopback") }
    var port by rememberSaveable(selector) { mutableStateOf("") }
    var localPort by rememberSaveable(selector) { mutableStateOf("0") }
    var busy by remember(selector) { mutableStateOf(false) }
    var error by remember(selector) { mutableStateOf<String?>(null) }
    var info by remember(selector) { mutableStateOf<String?>(null) }
    var reload by remember(selector) { mutableIntStateOf(0) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(selector, reload) {
        try {
            targets = TcpApi.devices().filter { target ->
                (selector.contactId != null && target.relation == "direct" && target.relationId == selector.contactId) ||
                    (selector.deviceId != null && selector.publicKey != null && target.deviceId == selector.deviceId && target.publicKey == selector.publicKey)
            }
            // The display follows the background service; this loop only reads local state.
            while (true) {
                val identity = targets.firstOrNull()
                if (identity != null) services = TcpApi.services(identity.deviceId, identity.publicKey)
                tunnels = TcpApi.list().filter { tunnel -> targets.any { it.nodeId.isNotEmpty() && it.nodeId == tunnel.remoteNode } }
                delay(2000)
            }
        } catch (cancel: CancellationException) { throw cancel
        } catch (e: CoreException) { error = "TCP-Status nicht geladen: ${e.message ?: e.kind}" }
    }
    val selected = targets.firstOrNull { it.present && it.canStart } ?: targets.firstOrNull { it.canStart }
    val portValue = port.toIntOrNull()?.takeIf { it in 1..65535 }
    val localValue = localPort.toIntOrNull()?.takeIf { it in 0..65535 }
    val valid = selected != null && selected.nodeId.isNotEmpty() && host.isNotBlank() && portValue != null && localValue != null
    fun act(failure: String, action: suspend () -> Unit) {
        if (busy) return
        busy = true; error = null
        scope.launch {
            try { action(); reload++ }
            catch (cancel: CancellationException) { throw cancel }
            catch (e: CoreException) { error = "$failure: ${e.message ?: e.kind}" }
            finally { busy = false }
        }
    }
    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        title = { Text("TCP: ${selector.name.ifBlank { "Gerät" }}") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Dienste dieses Geräts", style = MaterialTheme.typography.titleSmall)
                val shared = services
                when {
                    shared == null -> Text("Dienste werden gelesen …")
                    else -> {
                        shared.unavailable?.let { Text(it) }
                        if (!shared.control && shared.unavailable == null) Text("Die Dienste dieses Geräts erscheinen hier, sobald es dir „Bildschirm steuern“ erlaubt.")
                        if (shared.control && shared.services.isEmpty() && shared.unavailable == null) Text("Auf diesem Gerät lauscht kein freigegebener Dienst.")
                        if (shared.services.isNotEmpty()) Text(TCP_ANY_APP_NOTICE, style = MaterialTheme.typography.bodySmall)
                        shared.services.forEach { service ->
                            val open = service.mapping
                            val name = service.process ?: "Port ${service.port}"
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                Text("$name · Port ${service.port}", modifier = Modifier.weight(1f))
                                val target = selected
                                if (open == null) {
                                    TextButton(enabled = !busy && target != null, onClick = {
                                        val device = target ?: return@TextButton
                                        act("Dienst nicht geöffnet") {
                                            val mapping = TcpApi.openService(device.deviceId, device.publicKey, service.port)
                                            info = "$name: ${mapping.url ?: mapping.address.orEmpty()}"
                                        }
                                    }) { Text("Öffnen") }
                                } else {
                                    TextButton(enabled = !busy && target != null, onClick = {
                                        val device = target ?: return@TextButton
                                        act("Nicht geschlossen") { TcpApi.closeService(device.deviceId, device.publicKey, service.port) }
                                    }) { Text("Schließen") }
                                }
                            }
                            open?.let { mapping ->
                                SelectionContainer { Text(mapping.url ?: mapping.address.orEmpty()) }
                                mapping.portNote?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                                mapping.lastError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                            }
                        }
                    }
                }
                if (selected == null) Text("Gerät ist nicht erreichbar")
                Text("Weitere Weiterleitung (Ziel im Netz des Geräts oder nicht freigegebener Port)", style = MaterialTheme.typography.titleSmall)
                OutlinedTextField(host, { host = it }, enabled = !busy, singleLine = true,
                    label = { Text("Zielhost: loopback, IP oder DNS") }, modifier = Modifier.fillMaxWidth())
                listOf(Triple("Zielport (1–65535)", port, { value: String -> port = value }),
                    Triple("Lokaler Port (0 = automatisch)", localPort, { value: String -> localPort = value })).forEach { (label, value, change) ->
                    OutlinedTextField(value, change, enabled = !busy, singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        label = { Text(label) }, modifier = Modifier.fillMaxWidth())
                }
                Text("Das Gerät muss diese Weiterleitung erlauben (Recht „TCP-Weiterleitung“); fragt es nach, wartet der Start auf die Antwort. Die Weiterleitung hat keine feste Frist.")
                TextButton(enabled = valid && !busy, onClick = {
                    val target = selected ?: return@TextButton
                    val targetPort = portValue ?: return@TextButton
                    val listenPort = localValue ?: return@TextButton
                    val targetHost = host.trim()
                    act("Weiterleitung nicht gestartet") {
                        val tunnel = TcpApi.start(target, targetHost, targetPort, listenPort)
                        info = "Weiterleitung ${tunnel.id}: ${tunnel.listener.orEmpty()}"
                    }
                }) { Text(if (busy) "TCP-Anfrage läuft …" else "Weiterleitung starten") }
                info?.let { SelectionContainer { Text(it) } }
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                if (tunnels.isNotEmpty()) Text("Laufende Weiterleitungen", style = MaterialTheme.typography.titleSmall)
                tunnels.forEach { tunnel ->
                    SelectionContainer {
                        Column {
                            Text(if (tunnel.role == "listener") "Lokaler Zugang ${tunnel.listener ?: "–"}" else "Zieldienst auf diesem Telefon")
                            Text("Ziel: ${destination(tunnel.destination)} · ${if (tunnel.direction == "peer_to_host") "dieses Telefon → Gerät" else "Gerät → dieses Telefon"}")
                            Text(lifetime(tunnel.expiresAt))
                            Text("${tunnel.activeConnections} Verbindungen · ${tunnel.socketReadBytes} B gelesen / ${tunnel.socketWrittenBytes} B geschrieben")
                            if (tunnel.closing) Text("Wird geschlossen …")
                            tunnel.lastError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                        }
                    }
                    TextButton(enabled = !busy && !tunnel.closing, onClick = { act("Nicht geschlossen") { TcpApi.close(tunnel.id) } }) { Text("Weiterleitung schließen") }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss, enabled = !busy) { Text("Schließen") } },
    )
}

private fun lifetime(expiresAt: Long?): String = when (expiresAt) {
    null -> "Ohne feste Frist"
    else -> "Endet ${Format.dateTime(expiresAt * 1000)} (ausdrücklich gewählte Frist)"
}
private fun destination(value: JsonObject): String {
    val host = value["host"] as? JsonObject
    fun field(key: String) = (host?.get(key) as? JsonPrimitive)?.content.orEmpty()
    val name = when (field("kind")) { "loopback" -> "Loopback des Geräts"; "dns" -> field("name"); else -> field("address") }
    return "$name:${(value["port"] as? JsonPrimitive)?.content.orEmpty()}"
}
