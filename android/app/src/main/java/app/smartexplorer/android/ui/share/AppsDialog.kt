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
import androidx.compose.material3.CircularProgressIndicator
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
import app.smartexplorer.android.api.FamilyApi
import app.smartexplorer.android.api.ConfirmedRemoteAppWindow
import app.smartexplorer.android.api.RemoteAppCatalog
import app.smartexplorer.android.api.RemoteAppReceipt
import app.smartexplorer.android.api.SavedRemoteApp
import app.smartexplorer.android.api.TcpApi
import app.smartexplorer.android.api.TcpTarget
import app.smartexplorer.android.screen.HostCatalogApp
import app.smartexplorer.android.ui.screen.ScreenTarget
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Programs of a device (design 6, user rule r3 „der rest muss laufen“): the catalog loads by itself,
 * an unclear start is clarified with the same receipt whenever the device comes back or its windows
 * change, a start that waits for the locked host session starts again once it is unlocked, and a
 * confirmed window opens by itself. Remote apps share only the saved device relationship, never
 * Exec/Screen rights; the double-start guard stays. While the host session is locked,
 * „Bildschirm steuern“ opens the device's screen through [onOpenScreen] (the screen's own rights
 * apply); the dialog stays open underneath, so the waiting start follows after the unlock.
 */
@Composable
internal fun AppsDialog(
    selector: ShareDialog.Apps,
    onDismiss: () -> Unit,
    onOpenWindow: ((ConfirmedRemoteAppWindow) -> Unit)? = null,
    onOpenScreen: ((ScreenTarget) -> Unit)? = null,
) {
    val context = LocalContext.current.applicationContext
    val scope = rememberCoroutineScope()
    var targets by remember(selector) { mutableStateOf<List<TcpTarget>>(emptyList()) }
    var selectedKey by remember(selector) { mutableStateOf("") }
    var hostEvent by remember(selector) { mutableStateOf<Pair<Boolean, List<String>>?>(null) }
    var relaunch by remember(selector) { mutableStateOf<String?>(null) }
    var openedRequest by remember(selector) { mutableStateOf<String?>(null) }
    var catalog by remember(selector) { mutableStateOf<RemoteAppCatalog?>(null) }
    var saved by remember(selector) { mutableStateOf<SavedRemoteApp?>(null) }
    var result by remember(selector) { mutableStateOf<AppReceiptResult?>(null) }
    var busy by remember(selector) { mutableStateOf(false) }
    var pending by remember(selector) { mutableStateOf<String?>(null) }
    var error by remember(selector) { mutableStateOf<String?>(null) }
    var reload by remember(selector) { mutableIntStateOf(0) }
    var search by remember(selector) { mutableStateOf("") }
    var page by remember(selector) { mutableIntStateOf(0) }
    LaunchedEffect(selector) {
        busy = true; pending = "Gespeicherte Gerätebeziehungen laden …"
        try {
            targets = TcpApi.devices().filter { target ->
                target.canStart && ((selector.contactId != null && target.relation == "direct" && target.relationId == selector.contactId) ||
                    (selector.deviceId != null && selector.publicKey != null && target.deviceId == selector.deviceId && target.publicKey == selector.publicKey) ||
                    (selector.endpoint != null && selector.endpoint == endpoint(target)))
            }
            selectedKey = (targets.firstOrNull { it.present } ?: targets.firstOrNull())?.key.orEmpty(); error = null
        } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { error = "Geräteliste nicht verfügbar: ${failure.message}" } finally { busy = false; pending = null }
    }
    val selected = targets.firstOrNull { it.key == selectedKey }
    LaunchedEffect(selectedKey, reload) {
        val target = selected ?: return@LaunchedEffect
        busy = true; pending = "Programmkatalog wird geladen …"; catalog = null; result = null; saved = null; page = 0
        try {
            var journalError: String? = null
            try { saved = AppsApi.load(context, target) } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { journalError = "Gespeicherter Auftrag nicht übernommen: ${failure.message}" }
            catalog = AppsApi.catalog(target); error = journalError
        } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { error = "Programmkatalog oder gespeicherter Auftrag nicht verfügbar: ${failure.message}" } finally { busy = false; pending = null }
    }
    // Not final yet: se asks the same receipt again on every host event (design 6 F4).
    val unresolved = saved != null && (result?.receipt == null || result?.receipt?.state in setOf("outcome_unknown", "started_window_pending"))
    val locked = result?.receipt?.state == "consent_required" && catalog?.session?.unlocked != true
    val unlockScreen = onOpenScreen?.let { lockedSessionScreen(selected, locked, catalog?.session?.unlocked) }
    fun query() {
        if (busy) return
        val target = selected ?: return
        val request = saved?.request ?: return
        busy = true
        scope.launch {
            busy = true; pending = "Startergebnis wird geklärt …"
            try {
                result = AppsApi.receipt(target, request)
                error = if (result?.receipt == null) "Startergebnis wird geklärt: noch kein Startbeleg; se fragt erneut, sobald das Gerät wieder erscheint oder sich seine Fenster ändern." else null
            } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { error = "Startergebnis wird geklärt; se fragt erneut, sobald das Gerät wieder erscheint: ${failure.message}" } finally { busy = false; pending = null }
        }
    }
    // Host events from this phone's background state: presence and the device's open windows.
    LaunchedEffect(selectedKey) {
        val target = selected ?: return@LaunchedEffect
        while (true) {
            try {
                val feed = FamilyApi.feed(target.deviceId, target.publicKey)
                hostEvent = Pair(feed.present, feed.families?.entries.orEmpty().map { it.source })
            } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { hostEvent = Pair(false, emptyList()) }
            delay(2000)
        }
    }
    // Immediately after an unclear result, then on every host event; never on a timer.
    LaunchedEffect(hostEvent, saved, result) {
        if (hostEvent?.first == true && unresolved) query()
    }
    // A start waiting for the locked session reads the catalog again on each host event.
    LaunchedEffect(hostEvent) {
        if (hostEvent?.first == true && relaunch != null && !busy) reload++
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
                result = null; relaunch = null; openedRequest = null
                try {
                    result = AppsApi.launch(target, request)
                } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) {
                    pending = "Startantwort unklar; ausschließlich Beleg derselben ID abfragen …"
                    result = AppsApi.receipt(target, request)
                    if (result?.receipt == null) error = "Startergebnis wird geklärt; se fragt erneut, sobald das Gerät wieder erscheint. ${failure.message}"
                }
                if (result?.receipt?.state == "consent_required" && !snapshot.session.unlocked) relaunch = app.appId
            } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { error = "Start nicht bestätigt: ${failure.message}" } finally { busy = false; pending = null }
        }
    }
    // A start that waited for the locked session runs again with a new request (design 6 F5).
    LaunchedEffect(catalog, relaunch) {
        val app = relaunch ?: return@LaunchedEffect
        val snapshot = catalog ?: return@LaunchedEffect
        if (!snapshot.session.unlocked || busy) return@LaunchedEffect
        relaunch = null
        snapshot.apps.firstOrNull { it.appId == app }?.let { start(it) }
    }
    // A confirmed window opens by itself once (design 6 F6).
    val confirmedWindow = result?.window
    LaunchedEffect(confirmedWindow) {
        val window = confirmedWindow ?: return@LaunchedEffect
        val opener = onOpenWindow ?: return@LaunchedEffect
        val requestId = window.request.requestId
        if (openedRequest == requestId) return@LaunchedEffect
        openedRequest = requestId
        opener(window)
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Programme · ${selector.name}") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (targets.isEmpty() && !busy) Text("Gerät ist nicht erreichbar")
                if (busy) Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    CircularProgressIndicator(Modifier.size(20.dp)); Text(pending ?: "Programmanfrage läuft …")
                }
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                saved?.let { attempt ->
                    Text("Letzter Start: ${attempt.name}", style = MaterialTheme.typography.titleSmall)
                    SelectionContainer { Text("Request-ID: ${attempt.request.requestId}") }
                    result?.receipt?.let { ReceiptDetails(it, locked) } ?: Text("Startergebnis wird geklärt")
                    if (locked && unlockScreen != null) UnlockScreenButton { onOpenScreen?.invoke(unlockScreen) }
                    result?.window?.let {
                        if (onOpenWindow != null) Text("Programmfenster wurde geöffnet.")
                        else Text("Fensterbindung bestätigt; der Anschluss an die aktuelle Fernfensterauswahl ist noch nicht verfügbar.")
                    }
                }
                if (unresolved) Text("Startergebnis wird geklärt")
                catalog?.let { snapshot ->
                    Text("Benutzer: ${snapshot.session.userId}\nSitzung: ${snapshot.session.sessionId}", style = MaterialTheme.typography.bodySmall)
                    snapshot.limits.forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
                    if (!snapshot.session.interactive || !snapshot.session.unlocked) Text("Die Hostsitzung ist nicht interaktiv oder gesperrt; ein Start wartet, bis sie entsperrt ist.")
                    if (!locked && unlockScreen != null) UnlockScreenButton { onOpenScreen?.invoke(unlockScreen) }
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
                            ProgramRow(target, snapshot, app, !busy && !unresolved, ::start)
                        }
                    } }
                }
            }
        },
        confirmButton = { TextButton(enabled = selected != null && !busy, onClick = { reload++ }) { Text("Neu laden") } },
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
private fun ReceiptDetails(receipt: RemoteAppReceipt, locked: Boolean) {
    Text(when (receipt.state) {
        "outcome_unknown" -> "Startergebnis wird geklärt"
        "started_window_pending" -> "Gestartet; Fenster noch nicht verfügbar"
        "started" -> "Gestartet"
        "activated" -> "Aktiviert"
        "consent_required" -> if (locked) "Hostsitzung gesperrt – zum Entsperren den Bildschirm öffnen; der Start folgt danach von selbst" else "Lokale Zustimmung am Zielgerät erforderlich"
        "denied" -> "Start verweigert"
        "unsupported" -> "Start auf dem Zielgerät nicht unterstützt"
        else -> "Unbekannter Hoststatus: ${receipt.state}"
    })
    Text(receipt.reason)
    if (receipt.window == null && receipt.state in setOf("started", "started_window_pending", "activated")) {
        Text("Noch keine bestätigte Bindung an ein Fernfenster vorhanden. Der Startbeleg enthält keinen auswählbaren Fensterbeleg.")
    }
}
@Composable
private fun UnlockScreenButton(onClick: () -> Unit) {
    TextButton(onClick = onClick) { Text("Bildschirm steuern") }
}

/**
 * The screen to open next to a locked host session: the selected device with control, pinned to its
 * current node key; `null` while the session is unlocked or no device is selected.
 */
internal fun lockedSessionScreen(target: TcpTarget?, locked: Boolean, sessionUnlocked: Boolean?): ScreenTarget? {
    if (target == null || (!locked && sessionUnlocked != false)) return null
    return ScreenTarget(
        relation = target.relation,
        relationId = target.relationId,
        deviceId = target.deviceId.ifBlank { null },
        control = true,
        expectedNodeId = target.nodeId.ifBlank { null },
    )
}

private fun endpoint(target: TcpTarget): String? = when (target.relation) {
    "direct" -> "share://direct/${target.relationId}"
    "room" -> "share://room/${target.relationId}/${target.deviceId}"
    else -> null
}
