package app.smartexplorer.android.ui.share

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.AppReceiptResult
import app.smartexplorer.android.api.AppsApi
import app.smartexplorer.android.api.ConfirmedRemoteAppWindow
import app.smartexplorer.android.api.RemoteAppCatalog
import app.smartexplorer.android.api.RemoteAppReceipt
import app.smartexplorer.android.api.SavedRemoteApp
import app.smartexplorer.android.api.TcpApi
import app.smartexplorer.android.api.TcpTarget
import app.smartexplorer.android.screen.HostCatalogApp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** Remote apps share only the saved device relationship, never Exec/Screen rights. */
@Composable
internal fun AppsDialog(
    selector: ShareDialog.Apps,
    onDismiss: () -> Unit,
    onOpenWindow: ((ConfirmedRemoteAppWindow) -> Unit)? = null,
) {
    val context = LocalContext.current.applicationContext
    val scope = rememberCoroutineScope()
    var targets by remember(selector) { mutableStateOf<List<TcpTarget>>(emptyList()) }
    var selectedKey by remember(selector) { mutableStateOf("") }
    var catalog by remember(selector) { mutableStateOf<RemoteAppCatalog?>(null) }
    var saved by remember(selector) { mutableStateOf<SavedRemoteApp?>(null) }
    var result by remember(selector) { mutableStateOf<AppReceiptResult?>(null) }
    var busy by remember(selector) { mutableStateOf(false) }
    var pending by remember(selector) { mutableStateOf<String?>(null) }
    var error by remember(selector) { mutableStateOf<String?>(null) }
    var reload by remember(selector) { mutableIntStateOf(0) }
    var search by remember(selector) { mutableStateOf("") }
    var page by remember(selector) { mutableIntStateOf(0) }
    var explicitNew by remember(selector) { mutableStateOf(false) }
    LaunchedEffect(selector) {
        busy = true; pending = "Gespeicherte Gerätebeziehungen laden …"
        try {
            targets = TcpApi.devices().filter { target ->
                target.canStart && ((selector.contactId != null && target.relation == "direct" && target.relationId == selector.contactId) ||
                    (selector.deviceId != null && selector.publicKey != null && target.deviceId == selector.deviceId && target.publicKey == selector.publicKey) ||
                    (selector.endpoint != null && selector.endpoint == endpoint(target)))
            }
            selectedKey = targets.firstOrNull()?.key.orEmpty(); error = null
        } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { error = "Geräteliste nicht verfügbar: ${failure.message}" } finally { busy = false; pending = null }
    }
    val selected = targets.firstOrNull { it.key == selectedKey }
    LaunchedEffect(selectedKey, reload) {
        val target = selected ?: return@LaunchedEffect
        busy = true; pending = "Programmkatalog abfragen …"; catalog = null; result = null; saved = null; explicitNew = false; page = 0
        try {
            var journalError: String? = null
            try { saved = AppsApi.load(context, target) } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { journalError = "Gespeicherter Auftrag nicht übernommen: ${failure.message}" }
            catalog = AppsApi.catalog(target); error = journalError
        } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { error = "Programmkatalog oder gespeicherter Auftrag nicht verfügbar: ${failure.message}" } finally { busy = false; pending = null }
    }
    val unresolved = saved != null && (result?.receipt == null || result?.receipt?.state in setOf("outcome_unknown", "consent_required"))
    fun query() {
        if (busy) return
        val target = selected ?: return
        val request = saved?.request ?: return
        busy = true
        scope.launch {
            busy = true; pending = "Ergebnis derselben gespeicherten Request-ID abfragen …"
            try {
                result = AppsApi.receipt(target, request)
                error = if (result?.receipt == null) "Noch kein Startbeleg für diese Request-ID. Kein automatischer Neustart." else null
            } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { error = "Startergebnis unklar; denselben Beleg erneut abfragen: ${failure.message}" } finally { busy = false; pending = null }
        }
    }
    fun start(app: HostCatalogApp) {
        if (busy) return
        val target = selected ?: return
        val snapshot = catalog ?: return
        busy = true
        scope.launch {
            busy = true; pending = "Startauftrag sicher speichern und einmal versenden …"; error = null
            try {
                val request = AppsApi.request(snapshot, app)
                // Save before IPC; recreation/process death can only recover this
                // exact request. No effect/recomposition ever invokes launch.
                saved = AppsApi.save(context, target, app.name, request)
                result = null; explicitNew = false
                try {
                    result = AppsApi.launch(target, request)
                } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) {
                    pending = "Startantwort unklar; ausschließlich Beleg derselben ID abfragen …"
                    result = AppsApi.receipt(target, request)
                    if (result?.receipt == null) error = "Noch kein Beleg; Start kann bereits erfolgt sein. ${failure.message}"
                }
            } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { error = "Start nicht bestätigt: ${failure.message}" } finally { busy = false; pending = null }
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Programme · ${selector.name}") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                targets.forEach { target ->
                    FilterChip(selected = selectedKey == target.key, enabled = !busy,
                        onClick = { selectedKey = target.key }, label = { Text("${target.relationLabel} / ${target.name}") })
                }
                if (targets.isEmpty() && !busy) Text("Keine gespeicherte ausgehende Beziehung zu dieser bestätigten Geräteidentität.")
                if (busy) Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    CircularProgressIndicator(Modifier.size(20.dp)); Text(pending ?: "Programmanfrage läuft …")
                }
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                saved?.let { attempt ->
                    Text("Letzter gespeicherter Auftrag: ${attempt.name}", style = MaterialTheme.typography.titleSmall)
                    SelectionContainer { Text("Request-ID: ${attempt.request.requestId}") }
                    result?.receipt?.let { ReceiptDetails(it) } ?: Text("Startstatus noch nicht bestätigt. Ausschließlich denselben Auftrag abfragen.")
                    TextButton(enabled = !busy, onClick = ::query) { Text("Ergebnis derselben Request-ID abfragen") }
                    result?.window?.let { window ->
                        if (onOpenWindow != null) TextButton(enabled = !busy, onClick = { onOpenWindow(window) }) { Text("Bestätigtes Appfenster öffnen") }
                        else Text("Fensterbindung bestätigt; der Anschluss an die aktuelle Fernfensterauswahl ist noch nicht verfügbar.")
                    }
                }
                if (unresolved) {
                    Text("Vor einem weiteren Start den gespeicherten Beleg abfragen. Eine lokale Zustimmung erfolgt ausschließlich am Zielgerät.")
                    Row {
                        Checkbox(checked = explicitNew, enabled = !busy, onCheckedChange = { explicitNew = it })
                        Text("Ich möchte ausdrücklich einen neuen Auftrag erstellen. Der vorige Start kann bereits erfolgt sein.")
                    }
                }
                catalog?.let { snapshot ->
                    Text("Benutzer: ${snapshot.session.userId}\nSitzung: ${snapshot.session.sessionId}", style = MaterialTheme.typography.bodySmall)
                    snapshot.limits.forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
                    if (!snapshot.session.interactive || !snapshot.session.unlocked) Text("Die aktuelle Hostsitzung ist nicht interaktiv oder gesperrt. Der Host kann Zustimmung verlangen oder den Start verweigern.")
                    OutlinedTextField(value = search, onValueChange = { search = it; page = 0 }, label = { Text("Programme suchen") }, singleLine = true)
                    val apps = snapshot.apps.filter { it.name.contains(search, ignoreCase = true) || it.appId.contains(search, ignoreCase = true) }
                    val pages = ((apps.size + 11) / 12).coerceAtLeast(1)
                    val current = page.coerceAtMost(pages - 1)
                    Row {
                        TextButton(enabled = current > 0, onClick = { page = current - 1 }) { Text("Zurück") }
                        Text("${current + 1} / $pages · ${apps.size}")
                        TextButton(enabled = current + 1 < pages, onClick = { page = current + 1 }) { Text("Weiter") }
                    }
                    if (apps.isEmpty()) Text("Keine für diese Beziehung sichtbaren Programme passen zur Suche.")
                    selected?.let { target -> apps.drop(current * 12).take(12).forEach { app ->
                        key(target.key, target.publicKey, snapshot.revision, app.appId, app.icon?.sha256) {
                            ProgramRow(target, snapshot, app, !busy && (!unresolved || explicitNew), ::start)
                        }
                    } }
                }
            }
        },
        confirmButton = { TextButton(enabled = selected != null && !busy, onClick = { reload++ }) { Text("Katalog aktualisieren") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Schließen") } },
    )
}

@Composable
private fun ProgramRow(target: TcpTarget, catalog: RemoteAppCatalog, app: HostCatalogApp, enabled: Boolean, start: (HostCatalogApp) -> Unit) {
    var image by remember { mutableStateOf<ImageBitmap?>(null) }
    var iconReason by remember { mutableStateOf<String?>(if (app.icon == null) "Kein bestätigtes Appsymbol im Katalog." else null) }
    LaunchedEffect(target.key, catalog.revision, app.appId, app.icon) {
        image = null
        iconReason = if (app.icon == null) "Kein bestätigtes Appsymbol im Katalog." else null
        try {
            image = AppsApi.icon(target, catalog, app)?.asImageBitmap()
            if (image == null && app.icon != null) iconReason = "Appsymbol ist am Zielgerät nicht mehr verfügbar."
        } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { iconReason = "Appsymbol nicht verfügbar: ${failure.message}" }
    }
    Card {
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                image?.let { Image(it, contentDescription = "Appsymbol für ${app.name}", modifier = Modifier.size(32.dp)) }
                    ?: Text("◇")
                Text(app.name, modifier = Modifier.weight(1f))
                TextButton(enabled = enabled && app.canLaunch, onClick = { start(app) }) { Text("Starten") }
            }
            if (!app.canLaunch) Text("Start nicht verfügbar: ${app.unavailableReason ?: "Host hat keine reale Startfähigkeit bestätigt."}")
            iconReason?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
        }
    }
}
@Composable
private fun ReceiptDetails(receipt: RemoteAppReceipt) {
    Text(when (receipt.state) {
        "outcome_unknown" -> "Startergebnis unklar; Beleg derselben ID erneut abfragen"
        "started_window_pending" -> "Gestartet; Fenster noch nicht verfügbar"
        "started" -> "Gestartet"
        "activated" -> "Aktiviert"
        "consent_required" -> "Lokale Zustimmung am Zielgerät erforderlich"
        "denied" -> "Start verweigert"
        "unsupported" -> "Start auf dem Zielgerät nicht unterstützt"
        else -> "Unbekannter Hoststatus: ${receipt.state}"
    })
    Text(receipt.reason)
    if (receipt.window == null && receipt.state in setOf("started", "started_window_pending", "activated")) {
        Text("Noch keine bestätigte Bindung an ein Fernfenster vorhanden. Der Startbeleg enthält keinen auswählbaren Fensterbeleg.")
    }
}
private fun endpoint(target: TcpTarget): String? = when (target.relation) {
    "direct" -> "share://direct/${target.relationId}"
    "room" -> "share://room/${target.relationId}/${target.deviceId}"
    else -> null
}
