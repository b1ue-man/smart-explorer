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
import androidx.compose.material3.Checkbox
import androidx.compose.material3.FilterChip
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
import app.smartexplorer.android.api.TCP_LOCAL_ACCESS_NOTICE
import app.smartexplorer.android.api.TcpApi
import app.smartexplorer.android.api.TcpTarget
import app.smartexplorer.android.api.TcpTunnel
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Format
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** Start and inspect exactly this device's Direct/room relationships. */
@Composable
internal fun TcpDialog(selector: ShareDialog.Tcp, onDismiss: () -> Unit) {
    var targets by remember(selector) { mutableStateOf<List<TcpTarget>>(emptyList()) }
    var tunnels by remember(selector) { mutableStateOf<List<TcpTunnel>>(emptyList()) }
    var selectedKey by rememberSaveable(selector) { mutableStateOf("") }
    var host by rememberSaveable(selector) { mutableStateOf("loopback") }
    var port by rememberSaveable(selector) { mutableStateOf("") }
    var localPort by rememberSaveable(selector) { mutableStateOf("0") }
    var lifetime by rememberSaveable(selector) { mutableStateOf("900") }
    var acknowledged by remember(selector) { mutableStateOf(false) }
    var busy by remember(selector) { mutableStateOf(false) }
    var error by remember(selector) { mutableStateOf<String?>(null) }
    var info by remember(selector) { mutableStateOf<String?>(null) }
    var reload by remember(selector) { mutableIntStateOf(0) }
    var now by remember { mutableStateOf(System.currentTimeMillis()) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(selector, reload) {
        try {
            targets = TcpApi.devices().filter { target ->
                (selector.contactId != null && target.relation == "direct" && target.relationId == selector.contactId) ||
                    (selector.deviceId != null && selector.publicKey != null && target.deviceId == selector.deviceId && target.publicKey == selector.publicKey)
            }
            if (targets.none { it.key == selectedKey && it.canStart }) selectedKey = targets.firstOrNull { it.canStart }?.key.orEmpty()
            while (true) {
                tunnels = TcpApi.list().filter { tunnel -> targets.any { it.nodeId.isNotEmpty() && it.nodeId == tunnel.remoteNode } }
                now = System.currentTimeMillis()
                delay(2000)
            }
        } catch (e: CoreException) { error = "TCP-Status nicht geladen: ${e.message ?: e.kind}" }
    }
    LaunchedEffect(selectedKey) { acknowledged = false }
    val selected = targets.firstOrNull { it.key == selectedKey && it.canStart }
    val portValue = port.toIntOrNull()?.takeIf { it in 1..65535 }
    val localValue = localPort.toIntOrNull()?.takeIf { it in 0..65535 }
    val lifetimeValue = lifetime.toIntOrNull()?.takeIf { it in 1..86400 }
    val valid = selected != null && selected.nodeId.isNotEmpty() && host.isNotBlank() && portValue != null && localValue != null && lifetimeValue != null && acknowledged
    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        title = { Text("TCP: ${selector.name.ifBlank { "Gerät" }}") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Beziehung für den neuen Zugang", style = MaterialTheme.typography.titleSmall)
                targets.filter { it.canStart }.forEach { target ->
                    FilterChip(selected = selectedKey == target.key, enabled = !busy, onClick = { selectedKey = target.key; acknowledged = false },
                        label = { Text("${target.relationLabel} / ${target.name}") })
                }
                if (selected == null) Text("Keine ausgehende Verbindung zu diesem Gerät gespeichert. Eingehende Weiterleitungen kannst du unten schließen.")
                selected?.let { Text("Zielgerät: ${it.deviceId}\nVerbindungsidentität: ${it.nodeId}\nBeziehung: ${it.relationId}") }
                OutlinedTextField(host, { host = it; acknowledged = false }, enabled = !busy, singleLine = true,
                    label = { Text("Zielhost: loopback, IP oder DNS") }, modifier = Modifier.fillMaxWidth())
                listOf(Triple("Zielport (1–65535)", port, { value: String -> port = value; acknowledged = false }),
                    Triple("Lokaler Port (0 = automatisch)", localPort, { value: String -> localPort = value; acknowledged = false }),
                    Triple("Lebensdauer in Sekunden (1–86400)", lifetime, { value: String -> lifetime = value; acknowledged = false })).forEach { (label, value, change) ->
                    OutlinedTextField(value, change, enabled = !busy, singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        label = { Text(label) }, modifier = Modifier.fillMaxWidth())
                }
                Text(TCP_LOCAL_ACCESS_NOTICE)
                Row {
                    Checkbox(acknowledged, { acknowledged = it }, enabled = !busy)
                    Text("Ich bestätige diesen lokalen Zugriff.")
                }
                Text("Das Zielgerät muss Host, Port und Richtung in genau dieser Beziehung erlauben. Die Gegenrichtung wird dort separat gestartet.")
                Text("Der Start wartet bis zu 30 Sekunden auf die Bestätigung; danach wird dieser Start abgebrochen.")
                TextButton(enabled = valid && !busy, onClick = {
                    if (!valid || busy) return@TextButton
                    val target = selected ?: return@TextButton
                    val targetPort = portValue ?: return@TextButton
                    val listenPort = localValue ?: return@TextButton
                    val seconds = lifetimeValue ?: return@TextButton
                    val targetHost = host.trim()
                    val localAccessConfirmed = acknowledged
                    busy = true; error = null; acknowledged = false
                    scope.launch {
                        try {
                            val tunnel = TcpApi.start(target, targetHost, targetPort, listenPort, seconds, acknowledgeLocalAccess = localAccessConfirmed)
                            info = "Weiterleitung ${tunnel.id}: ${tunnel.listener.orEmpty()}"
                            reload++
                        } catch (e: CoreException) { error = "Weiterleitung nicht gestartet: ${e.message ?: e.kind}" }
                        finally { busy = false }
                    }
                }) { Text(if (busy) "TCP-Anfrage läuft …" else "Weiterleitung starten") }
                info?.let { SelectionContainer { Text(it) } }
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                Text("Aktive Weiterleitungen", style = MaterialTheme.typography.titleSmall)
                if (tunnels.isEmpty()) Text("Keine aktive Weiterleitung für dieses Gerät.")
                tunnels.forEach { tunnel ->
                    SelectionContainer {
                        Column {
                            Text("ID: ${tunnel.id}")
                            Text(if (tunnel.role == "listener") "Lokaler Zugang" else "Zieldienst auf diesem Telefon")
                            Text("Initiator: ${tunnel.initiatorNode}\nZielgerät: ${tunnel.targetNode}")
                            Text("Beziehung: ${tunnel.relationKind} / ${tunnel.relationId}\nRichtung: ${if (tunnel.direction == "peer_to_host") "Anfragendes Gerät → Zielhost" else "Host → Gegenstelle"}")
                            Text("Zieldienst: ${destination(tunnel.destination)}\nGebundenes Ziel: ${tunnel.boundTarget ?: "–"}\nLokaler Listener: ${tunnel.listener ?: "–"}")
                            val remaining = ((tunnel.expiresAt * 1000 - now) / 1000).coerceAtLeast(0)
                            Text("Spätester Ablauf: ${Format.dateTime(tunnel.expiresAt * 1000)} (noch $remaining s)")
                            Text("${tunnel.activeConnections} Verbindungen · ${tunnel.socketReadBytes} B gelesen / ${tunnel.socketWrittenBytes} B geschrieben")
                            if (tunnel.closing) Text("Wird geschlossen …")
                            tunnel.lastError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                        }
                    }
                    TextButton(enabled = !busy && !tunnel.closing, onClick = {
                        busy = true; error = null
                        scope.launch {
                            try { TcpApi.close(tunnel.id); reload++ }
                            catch (e: CoreException) { error = "Nicht geschlossen: ${e.message ?: e.kind}"; reload++ }
                            finally { busy = false }
                        }
                    }) { Text("Weiterleitung schließen") }
                }
            }
        },
        confirmButton = { TextButton(onClick = { reload++ }, enabled = !busy) { Text("Status aktualisieren") } },
        dismissButton = { TextButton(onClick = onDismiss, enabled = !busy) { Text("Schließen") } },
    )
}
private fun destination(value: JsonObject): String {
    val host = value["host"] as? JsonObject
    fun field(key: String) = (host?.get(key) as? JsonPrimitive)?.content.orEmpty()
    val name = when (field("kind")) { "loopback" -> "Loopback des Zielgeräts"; "dns" -> field("name"); else -> field("address") }
    return "$name:${(value["port"] as? JsonPrimitive)?.content.orEmpty()}"
}
