package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import app.smartexplorer.android.api.ShareApi
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SectionHeader
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.settings.SettingsStatusRow
import app.smartexplorer.android.ui.settings.SettingsValue
import kotlinx.coroutines.delay
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.longOrNull

/** Stable identity selector; names are only headings and never select a policy. */
@Composable
internal fun DeviceSettingsPage(selector: ShareDialog.DeviceSettings, vm: ShareViewModel, onDismiss: () -> Unit) {
    var snapshot by remember(selector) { mutableStateOf<JsonObject?>(null) }
    var loading by remember(selector) { mutableStateOf(false) }
    var error by remember(selector) { mutableStateOf<String?>(null) }
    var reload by remember(selector) { mutableIntStateOf(0) }
    var fetched by remember(selector) { mutableIntStateOf(0) }
    var child by remember(selector) { mutableStateOf<ShareDialog?>(null) }
    var diagnostics by rememberSaveable { mutableStateOf(false) }
    val status = vm.status
    LaunchedEffect(selector, reload, status?.execTargets, status?.rooms, status?.writeGrants, status?.exports) {
        if (selector.contactId == null && (selector.deviceId == null || selector.publicKey == null)) {
            error = "Die Geräteidentität ist noch nicht bestätigt. Zuerst das Mitglied ausdrücklich zulassen."
            return@LaunchedEffect
        }
        loading = true
        try {
            snapshot = ShareApi.deviceRights(selector.contactId, selector.deviceId, selector.publicKey)
            fetched++
            error = null
        } catch (e: CoreException) {
            snapshot = null
            error = "Rechtequellen nicht geladen: ${e.message ?: e.kind}"
        }
        finally { loading = false }
    }
    LaunchedEffect(snapshot, fetched) {
        val deadlines = snapshot?.obj("rights")?.objects("relations").orEmpty().flatMap(::sourceDeadlines)
        val now = System.currentTimeMillis() / 1000
        val next = deadlines.filter { it > now }.minOrNull() ?: return@LaunchedEffect
        delay(((next - now) * 1000).coerceAtLeast(1))
        reload++
    }
    val contactId = snapshot?.string("contactId") ?: selector.contactId
    val device = status?.devices?.firstOrNull { it.contactId == contactId }
    val deviceId = snapshot?.string("deviceId") ?: selector.deviceId
    val publicKey = snapshot?.string("publicKey") ?: selector.publicKey
    val grant = status?.writeGrants?.firstOrNull { it.deviceId == deviceId && it.publicKey == publicKey }
    val rights = snapshot?.obj("rights")
    val relations = rights?.objects("relations").orEmpty()
    val reachable = rights?.objects("reachable").orEmpty()
    val open: (ShareDialog) -> Unit = { child = it }
    val requests = status?.incoming.orEmpty().filter { contactId != null && it.contactId == contactId }
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        SubPageScaffold(title = device?.name?.ifBlank { "Gerät ohne Namen" } ?: selector.name.ifBlank { "Gerät ohne Namen" }, onBack = onDismiss) { padding ->
            LazyColumn(Modifier.fillMaxSize().padding(padding)) {
                item {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        SettingsStatusRow(loading = loading, error = error)
                        TextButton(onClick = { reload++ }, enabled = !loading) { Text("Rechte aktualisieren") }
                        TextButton(onClick = { open(ShareDialog.Tcp(selector.name, contactId, deviceId, publicKey)) }, enabled = !loading) { Text("TCP-Weiterleitungen …") }
                        TextButton(onClick = { open(ShareDialog.Rdp(selector.name, contactId, deviceId, publicKey)) }, enabled = !loading) { Text("Remote Desktop / RemoteApp …") }
                        TextButton(onClick = { open(ShareDialog.Apps(selector.name, contactId, deviceId, publicKey)) }, enabled = !loading) { Text("Programme auf diesem Gerät …") }
                        TextButton(onClick = { open(ShareDialog.Families(selector.name, contactId, deviceId, publicKey)) }, enabled = !loading) { Text("Einzelne Programmfenster …") }
                        TextButton(onClick = { open(ShareDialog.LoginSetup) }) { Text("Administrative Maschinenanmeldung …") }
                        if (deviceId != null && publicKey != null) TextButton(onClick = { open(ShareDialog.AppGrant(selector.name, deviceId, publicKey)) }, enabled = !loading && error == null) { Text("Eigene Programme für dieses Gerät freigeben …") }
                        Text("Jede Verbindung verwendet die Freigabe ihrer Beziehung. Der Gesamtzugang zeigt erreichbare Wege mit ihren Bedingungen.")
                        if (relations.any { it.string("relation_kind") == "room" && it.flag("active") && (hasFiles(it) || reachable.any { right -> right["source"] == it["target"] }) }) {
                            Text("Ein Direktentzug beendet den Raumzugang nicht. Verbleibende Raumrechte werden unten mit ihrer Quelle angezeigt.", color = MaterialTheme.colorScheme.error)
                        }
                    }
                }
                if (device != null) item {
                    SectionHeader("Verbindung und Rückfreigabe")
                    FlowRow(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(onClick = { open(ShareDialog.DeviceConnection(device)) }) { Text("Automatisch verbinden / öffnen …") }
                        TextButton(onClick = { open(ShareDialog.ShareBack(device)) }) { Text("Eigene Freigaben …") }
                        TextButton(onClick = { open(ShareDialog.RequestAccess(device.contactId, device.name)) }) { Text("Zugriff anfragen …") }
                        TextButton(onClick = { open(ShareDialog.ResetTrust(device)) }) { Text("Vertrauen zurücksetzen …") }
                        TextButton(onClick = { open(ShareDialog.RemoveDevice(device.contactId, device.name)) }) { Text("Gerät entfernen …") }
                    }
                }
                item { RouteDetails(contactId = contactId, deviceId = deviceId, publicKey = publicKey) }
                if (grant != null) item {
                    SectionHeader("Direktfreigabe für Dateien")
                    FlowRow(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(onClick = { open(ShareDialog.ContactWrite(grant)) }) { Text("Schreibrecht …") }
                        if (grant.active) TextButton(onClick = { open(ShareDialog.WithdrawGrant(grant)) }) { Text("Direktfreigabe entziehen …") }
                        else TextButton(onClick = { open(ShareDialog.AllowGrant(grant)) }) { Text("Direktfreigabe wieder zulassen …") }
                    }
                }
                item { SectionHeader("Rechte je Beziehung") }
                if (relations.isEmpty() && !loading && error == null) item { HintLine("Keine Freigabequelle für diese gepinnte Geräteidentität.") }
                items(relations, key = { it.string("targetKey").orEmpty() }) { source ->
                    Card(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
                        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                            Text(sourceName(source), style = MaterialTheme.typography.titleMedium)
                            Text(if (source.flag("active")) "Beziehung aktiv" else "Beziehung inaktiv – kein Zugang über diese Quelle")
                            val rows = sourceRows(source, reachable)
                            val labels = rows.map { it.first }
                            rows.forEach { (label, value) -> SettingsValue(label, value, labels) }
                            conditionLines(source).forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
                            val key = source.string("targetKey")
                            if (key != null) TextButton(onClick = { open(ShareDialog.AccessSource(key)) }, enabled = error == null && !loading) { Text("Diese Quelle bearbeiten …") }
                            if (deviceId != null && publicKey != null) TextButton(onClick = { open(ShareDialog.AppGrant(selector.name, deviceId, publicKey, source)) }, enabled = error == null && !loading) { Text("Eigene Appstarts für diese Quelle …") }
                            if (source.string("relation_kind") == "room") {
                                val room = status?.rooms?.firstOrNull { it.roomId == source.string("relation_id") }
                                if (room != null) TextButton(onClick = { open(ShareDialog.RoomPolicy(room)) }) { Text("Dateirechte dieses Raums …") }
                            }
                        }
                    }
                }
                item {
                    SectionHeader("Erreichbarer Gesamtzugang")
                    val fileSources = relations.filter { it.flag("active") && hasFiles(it) }
                    fileSources.forEach { HintLine("Dateien und Verbindungen über ${sourceName(it)} – Bereichsgrenzen stehen bei der Quelle.") }
                    reachable.forEach { right ->
                        val source = relations.firstOrNull { it["target"] == right["source"] }
                        HintLine("${capabilityName(right.string("capability"))}: ${decisionName(right.string("decision"))} über ${source?.let(::sourceName) ?: "nicht mehr verfügbare Quelle"}; die Bedingungen stehen bei dieser Quelle.")
                    }
                    if (reachable.isEmpty() && fileSources.isEmpty() && !loading) HintLine("Derzeit kein Zugang über eine aktive Beziehung.")
                }
                if (requests.isNotEmpty()) item {
                    SectionHeader("Offene Anfragen dieses Geräts")
                    requests.forEach { request ->
                        Text(request.stateText, Modifier.padding(horizontal = 16.dp))
                        FlowRow {
                            if (request.canAccept) TextButton(onClick = { vm.act("Anfrage nicht angenommen", onResult = { if (it == null) reload++ }) { ShareApi.decide(request.requestId, true) } }) { Text("Annehmen") }
                            if (request.canReject) TextButton(onClick = { vm.act("Anfrage nicht abgelehnt", onResult = { if (it == null) reload++ }) { ShareApi.decide(request.requestId, false) } }) { Text("Ablehnen") }
                        }
                    }
                }
                item {
                    TextButton(onClick = { diagnostics = !diagnostics }) { Text("Identität und Diagnose ${if (diagnostics) "schließen" else "anzeigen"}") }
                    if (diagnostics) SelectionContainer { Text(snapshot?.toString().orEmpty(), modifier = Modifier.padding(16.dp), style = MaterialTheme.typography.bodySmall) }
                }
            }
        }
        child?.let { current -> ShareDialogHost(current, vm,
            onDismiss = { child = null; reload++ }, onReplace = { child = it }, onShowTransfers = {}) }
    }
}

/** Includes devices with only incoming authorization, and room-only identities. */
@Composable
internal fun DeviceRightsIndex(status: ShareStatus?, open: (ShareDialog) -> Unit) {
    val identities = buildMap<Pair<String, String>, Pair<String, String>>() {
        status?.writeGrants.orEmpty().forEach { put(it.deviceId to it.publicKey, it.name to it.fingerprint) }
        status?.rooms.orEmpty().forEach { room -> room.members.forEach { member ->
            member.publicKey?.let { putIfAbsent(member.deviceId to it, member.name to member.fingerprint.orEmpty()) }
        } }
    }
    val withoutContact = identities.filterKeys { identity -> status?.devices.orEmpty().none { it.deviceId == identity.first && it.publicKey == identity.second } }
    if (withoutContact.isEmpty()) return
    SectionHeader("Geräte mit eingehenden Freigaben oder Raumzugang")
    withoutContact.forEach { (identity, display) ->
        TextButton(onClick = { open(ShareDialog.DeviceSettings(display.first, deviceId = identity.first, publicKey = identity.second)) }, modifier = Modifier.padding(horizontal = 8.dp)) {
            Text("${display.first.ifBlank { "Gerät ohne Namen" }} · Rechte und Beziehungen …")
        }
    }
}

private fun JsonObject.string(key: String): String? = (get(key) as? JsonPrimitive)?.takeIf { it != JsonNull }?.content
private fun JsonObject.flag(key: String): Boolean = (get(key) as? JsonPrimitive)?.booleanOrNull == true
private fun JsonObject.obj(key: String): JsonObject? = get(key) as? JsonObject
private fun JsonObject.objects(key: String): List<JsonObject> = (get(key) as? JsonArray)?.mapNotNull { it as? JsonObject }.orEmpty()
private fun JsonObject.strings(key: String): String = (get(key) as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.content }?.joinToString(", ").orEmpty()
private fun sourceName(source: JsonObject): String = if (source.string("relation_kind") == "direct") "Direktfreigabe" else "Raum „${source.string("relation_name").orEmpty()}“"
private fun decisionName(value: String?): String = when (value) { "allow" -> "Erlaubt"; "ask" -> "Mit Rückfrage"; else -> "Aus" }
private fun capabilityName(key: String?): String = when (key) {
    "exec_commands" -> "Befehle"; "exec_terminal" -> "Terminal"; "screen_view" -> "Bildschirm ansehen"; "screen_control" -> "Bildschirm steuern"
    "app_launch" -> "Apps starten"; "login_access" -> "Anmeldezugang"; "tcp_forward" -> "TCP weiterleiten"; else -> "Unbekannte Fähigkeit dieser App-Version"
}
private fun date(value: JsonElement?): String = (value as? JsonPrimitive)?.longOrNull?.let { Format.dateTime(it * 1000) } ?: "Unbegrenzt"
private fun hasFiles(source: JsonObject): Boolean = source.obj("exports")?.let { it.objects("roots").isNotEmpty() || it.objects("shared_connections").isNotEmpty() || it.flag("include_connections") } == true

private fun sourceRows(source: JsonObject, reachable: List<JsonObject>): List<Pair<String, String>> = buildList {
    val grant = source.obj("grant") ?: return@buildList
    val exports = source.obj("exports")?.objects("roots").orEmpty()
    add("Dateien lesen" to if (!hasFiles(source)) "Keine Ordner oder Verbindungen freigegeben" else "Freigegebene Ordner und Verbindungen")
    add("Dateien schreiben" to if (source.flag("may_write")) "Innerhalb der Schreibrechte jedes Ordners" else "Aus")
    val decisions = grant.obj("capabilities")
    val keys = linkedSetOf("exec_commands", "exec_terminal", "screen_view", "screen_control")
    keys.addAll(decisions?.keys.orEmpty())
    grant.obj("constraints")?.let { conditions ->
        if (conditions.obj("app_launch") != null) keys.add("app_launch")
        if (conditions.obj("login_access") != null) keys.add("login_access")
        if (conditions.objects("tcp_forward").isNotEmpty()) keys.add("tcp_forward")
    }
    keys.forEach { key ->
        val chosen = decisionName(decisions?.string(key))
        val current = reachable.firstOrNull { it["source"] == source["target"] && it.string("capability") == key }
        val effective = when {
            grant.flag("expired") -> "Abgelaufen"
            !source.flag("active") -> "Inaktiv"
            current != null -> decisionName(current.string("decision"))
            decisions?.string(key) == "off" || decisions?.string(key) == null -> "Aus"
            else -> "Nicht wirksam – keine aktuell gültige Bedingung"
        }
        add(capabilityName(key) to "$chosen · wirksam: $effective")
    }
    add("Konto" to grant.string("account_label").orEmpty())
    add("Gültigkeit" to date(grant["valid_until"]))
    exports.forEach { root -> add(root.string("label").orEmpty() to if (root.string("access") == "read_write" && source.flag("may_write")) "Lesen und Schreiben" else "Nur lesen") }
    source.obj("exports")?.objects("shared_connections")?.forEach { connection ->
        add("Verbindung ${connection.string("account").orEmpty()}" to if (connection.string("access") == "read_write" && source.flag("may_write")) "Lesen und Schreiben; Providerrechte gelten weiterhin" else "Nur lesen")
    }
    if (source.obj("exports")?.flag("include_connections") == true) add("Alte Verbindungsfreigabe" to "Gespeicherte Altregel; Freigaben dieses Geräts oder Raums prüfen")
}
private fun sourceDeadlines(source: JsonObject): List<Long> = buildList {
    val grant = source.obj("grant") ?: return@buildList
    (grant["valid_until"] as? JsonPrimitive)?.longOrNull?.let(::add)
    grant.obj("constraints")?.let { conditions ->
        val rules = listOfNotNull(conditions.obj("app_launch"), conditions.obj("login_access")) + conditions.objects("tcp_forward")
        rules.forEach { (it["valid_until"] as? JsonPrimitive)?.longOrNull?.let(::add) }
    }
}
private fun conditionLines(source: JsonObject): List<String> = buildList {
    val grant = source.obj("grant") ?: return@buildList
    val programs = source.strings("programs")
    if (programs.isNotEmpty()) add("Programme: $programs")
    val conditions = grant.obj("constraints")
    conditions?.obj("app_launch")?.let { add("Erlaubte Apps: ${it.strings("app_ids")} · Benutzer: ${it.strings("users")} · Sitzungen: ${it.strings("sessions")} · Ende: ${date(it["valid_until"])}") }
    conditions?.obj("login_access")?.let { add("Administrativ freigegebene Benutzer: ${it.strings("users")} · Sitzungen: ${it.strings("sessions")} · Ende: ${date(it["valid_until"])}") }
    conditions?.objects("tcp_forward")?.forEach { rule ->
        val host = rule.obj("host")
        val target = when (host?.string("kind")) { "loopback" -> "Loopback des Zielgeräts"; "ip" -> host.string("address"); "dns" -> "${host.string("name")} (${host.strings("addresses")})"; else -> "Ziel nicht bekannt" }
        val direction = if (rule.string("direction") == "peer_to_host") "Gegenstelle → dieses Gerät" else "Dieses Gerät → Gegenstelle"
        add("TCP: $direction · Ziel $target · Ports ${rule.strings("ports")} · Ende ${date(rule["valid_until"])}")
    }
    if (grant.flag("legacy_reader_disables_grant")) add("Eine ältere App ohne diese Einschränkungen deaktiviert diese Freigabe vollständig.")
}
