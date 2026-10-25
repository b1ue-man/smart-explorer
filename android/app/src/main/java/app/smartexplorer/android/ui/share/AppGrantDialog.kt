package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.rememberScrollState
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
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.AppsApi
import app.smartexplorer.android.api.LocalAppGrantChange
import app.smartexplorer.android.api.LocalAppGrantSnapshot
import app.smartexplorer.android.api.LocalAppRelation
import app.smartexplorer.android.ui.common.Format
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull

/** This phone's catalog; no remote command can obtain this first-grant source. */
@Composable
internal fun AppGrantDialog(selector: ShareDialog.AppGrant, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    var snapshot by remember(selector) { mutableStateOf<LocalAppGrantSnapshot?>(null) }
    var source by remember(selector) { mutableStateOf<LocalAppRelation?>(null) }
    var selected by remember(selector) { mutableStateOf<Set<String>>(emptySet()) }
    var decision by remember(selector) { mutableStateOf("off") }
    var expiry by remember(selector) { mutableIntStateOf(1) }
    var oldEnd by remember(selector) { mutableStateOf<Long?>(null) }
    var rebind by remember(selector) { mutableStateOf(false) }
    var acknowledged by remember(selector) { mutableStateOf(false) }
    var search by remember(selector) { mutableStateOf("") }
    var page by remember(selector) { mutableIntStateOf(0) }
    var busy by remember(selector) { mutableStateOf(false) }
    var fresh by remember(selector) { mutableStateOf(false) }
    var error by remember(selector) { mutableStateOf<String?>(null) }
    var info by remember(selector) { mutableStateOf<String?>(null) }
    var reload by remember(selector) { mutableIntStateOf(0) }
    fun choose(next: LocalAppRelation, current: LocalAppGrantSnapshot) {
        source = next; decision = next.grant.capabilities["app_launch"] ?: "off"; acknowledged = false
        val bounds = next.grant.constraints["app_launch"] as? JsonObject
        val priorIds = bounds?.strings("app_ids").orEmpty()
        selected = priorIds.intersect(current.catalog.apps.map { it.appId }.toSet())
        oldEnd = (bounds?.get("valid_until") as? JsonPrimitive)?.longOrNull
        expiry = if (bounds == null) 1 else 0
        rebind = bounds != null && (bounds.strings("sessions") != setOf(current.catalog.authorizationScope) ||
            bounds.strings("users") != setOf(current.catalog.session.userId) || selected != priorIds)
        error = null
    }
    LaunchedEffect(selector, reload) {
        busy = true; fresh = false; error = null
        val previous = source
        try {
            val loaded = AppsApi.localGrants(selector.deviceId, selector.publicKey)
            snapshot = loaded; fresh = true
            val expectedTarget = previous?.target ?: selector.source?.get("target") as? JsonObject
            val expectedRevision = previous?.policyRevision ?: (selector.source?.get("policy_revision") as? JsonPrimitive)?.longOrNull
            val next = loaded.relations.firstOrNull { it.target == expectedTarget }
            source = null; selected = emptySet(); acknowledged = false
            if (next != null && (reload > 0 || expectedRevision == null || next.policyRevision == expectedRevision)) choose(next, loaded)
            else if (expectedTarget != null) info = "Die angezeigte Rechtequelle wurde geändert. Die aktuelle Quelle unten ausdrücklich neu wählen."
        } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { error = "Lokaler Appkatalog nicht verfügbar: ${failure.message}" } finally { busy = false }
    }
    fun save() {
        if (busy) return
        val current = snapshot ?: return
        val chosen = source ?: return
        val end = when (expiry) { 0 -> oldEnd; 1 -> now() + 900; 2 -> now() + 3600; 3 -> now() + 28800; 4 -> now() + 86400; else -> null }
        val change = LocalAppGrantChange(chosen.target, chosen.policyRevision, current.catalog.deviceId,
            current.catalog.revision, current.catalog.session.userId, current.catalog.authorizationScope,
            selected.sorted(), decision, end)
        busy = true; error = null; info = null
        scope.launch {
            try {
                AppsApi.setLocalGrant(change)
                val loaded = AppsApi.localGrants(selector.deviceId, selector.publicKey)
                snapshot = loaded; fresh = true
                val next = loaded.relations.firstOrNull { it.target == chosen.target }
                if (next != null) choose(next, loaded) else { source = null; selected = emptySet() }
                info = "Appstartfreigabe dieser Quelle gespeichert. Befehls-, Terminal-, Bildschirm-, Anmelde- und TCP-Rechte bleiben erhalten."
            } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { fresh = false; error = "Appfreigabe nicht bestätigt: ${failure.message}" } finally { busy = false }
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Eigene Programme für ${selector.name}") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Diese Auswahl stammt aus der aktuellen Anmeldung auf diesem Telefon. Wähle genau eine Direkt- oder Raumfreigabe.")
                if (busy) Text("Lokaler Appkatalog oder Appfreigabe wird verarbeitet …")
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                info?.let { Text(it) }
                snapshot?.let { current ->
                    if (!fresh) Text("Katalog oder Rechtequelle sind nicht mehr aktuell. Vor dem Erlauben oder Fragen erneut laden; Ausschalten bleibt möglich.")
                    Text("Lokaler Appbenutzer: ${current.catalog.session.userId}")
                    if (current.relations.isEmpty()) Text("Keine eingehende Freigabequelle für diese Geräteidentität. Zuerst die Direktbeziehung oder das Raummitglied zulassen.")
                    current.relations.forEach { relation ->
                        FilterChip(selected = source?.target == relation.target, enabled = !busy,
                            onClick = { choose(relation, current); info = null }, label = { Text(if (relation.relationKind == "direct") "Direktfreigabe (${relation.relationId})" else "Raum „${relation.relationName}“ (${relation.relationId})") })
                    }
                    source?.let { chosen ->
                        Text("Policyrevision: ${chosen.policyRevision}", style = MaterialTheme.typography.bodySmall)
                        if (!chosen.active) Text("Beziehung inaktiv. Appstarts können ausschließlich ausgeschaltet werden.")
                        if (chosen.grant.expired) Text("Die übergeordnete Freigabe ist abgelaufen. Ihre gemeinsame Frist bleibt erhalten und begrenzt auch Appstarts.")
                        listOf("off" to "Aus", "ask" to "Vor jedem Start fragen", "allow" to "Ausgewählte Apps erlauben").forEach { (value, label) ->
                            FilterChip(selected = decision == value, enabled = !busy, onClick = { decision = value }, label = { Text(label) })
                        }
                        if (decision == "off") Text("Aus entzieht ausschließlich Appstarts. Bisherige Appgrenzen bleiben gespeichert; die neue Auswahl gilt erst beim ausdrücklichen Aktivieren.")
                        if (rebind) {
                            Text("Anmeldung, Katalog oder bisherige Appmenge geändert. Bisherige Rechte gelten nicht für den neuen Katalog.")
                            Row { Checkbox(checked = acknowledged, enabled = !busy, onCheckedChange = { acknowledged = it }); Text("Ausgewählte App-IDs ausdrücklich an diese aktuelle Anmeldung und diesen Katalog binden") }
                        }
                        Text("Gültigkeit dieser Appfreigabe", style = MaterialTheme.typography.titleSmall)
                        val labels = listOf("Bisherige Appgültigkeit beibehalten", "15 Minuten ab Speichern", "1 Stunde ab Speichern", "8 Stunden ab Speichern", "1 Tag ab Speichern", "Unbegrenzt für diese Anmeldung und diesen Katalog")
                        labels.forEachIndexed { index, label -> FilterChip(selected = expiry == index, enabled = !busy, onClick = { expiry = index }, label = { Text(label) }) }
                        Text("Bisheriges eigenes Ende: ${oldEnd?.let { Format.dateTime(it * 1000) } ?: "Unbegrenzt"}", style = MaterialTheme.typography.bodySmall)
                        OutlinedTextField(value = search, onValueChange = { search = it; page = 0 }, label = { Text("Lokale Programme suchen") }, singleLine = true)
                        val visible = current.catalog.apps.filter { it.name.contains(search, true) || it.appId.contains(search, true) }
                        val pages = ((visible.size + 11) / 12).coerceAtLeast(1)
                        val currentPage = page.coerceAtMost(pages - 1)
                        Row {
                            TextButton(enabled = currentPage > 0, onClick = { page = currentPage - 1 }) { Text("Zurück") }
                            Text("${currentPage + 1} / $pages · ${visible.size}")
                            TextButton(enabled = currentPage + 1 < pages, onClick = { page = currentPage + 1 }) { Text("Weiter") }
                        }
                        visible.drop(currentPage * 12).take(12).forEach { app ->
                            Row {
                                Checkbox(checked = app.appId in selected, enabled = !busy && (app.canLaunch || app.appId in selected),
                                    onCheckedChange = { yes -> selected = if (yes) selected + app.appId else selected - app.appId })
                                Column { Text(app.name); Text(app.appId, style = MaterialTheme.typography.bodySmall)
                                    if (!app.canLaunch) Text("Start nicht verfügbar: ${app.unavailableReason ?: "Keine reale Startfähigkeit bestätigt"}") }
                            }
                        }
                        Text("${selected.size} konkrete App-IDs ausgewählt. Neu installierte Programme werden nicht automatisch aufgenommen.")
                        val on = decision != "off"
                        val endValid = expiry != 0 || oldEnd?.let { it > now() } != false
                        val selectedAvailable = selected.all { id -> current.catalog.apps.any { it.appId == id && it.canLaunch } }
                        TextButton(enabled = !busy && (!on || (fresh && error == null && chosen.active && selected.isNotEmpty() && selectedAvailable && (!rebind || acknowledged) && endValid)), onClick = ::save) {
                            Text("Appstartfreigabe ausschließlich für diese Quelle speichern")
                        }
                    }
                }
            }
        },
        confirmButton = { TextButton(enabled = !busy, onClick = { reload++ }) { Text("Lokalen Katalog und Rechte aktualisieren") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Schließen") } },
    )
}
private fun JsonObject.strings(key: String): Set<String> = (get(key) as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull }?.toSet().orEmpty()
private fun now(): Long = System.currentTimeMillis() / 1000
