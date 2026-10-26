package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import app.smartexplorer.android.api.ConfirmedRemoteAppWindow
import app.smartexplorer.android.api.FamilyApi
import app.smartexplorer.android.api.TcpApi
import app.smartexplorer.android.api.TcpTarget
import app.smartexplorer.android.ui.screen.FamilyPicker
import app.smartexplorer.android.ui.screen.ScreenTarget
import kotlinx.coroutines.CancellationException

/** A saved relationship, including its current key, is selected before capture. */
@Composable
internal fun FamilyTargetsDialog(selector: ShareDialog.Families, onDismiss: () -> Unit, onReplace: (ShareDialog) -> Unit) {
    var targets by remember(selector) { mutableStateOf<List<TcpTarget>>(emptyList()) }
    var selected by remember(selector) { mutableStateOf<TcpTarget?>(null) }
    var loading by remember(selector) { mutableStateOf(true) }
    var error by remember(selector) { mutableStateOf<String?>(null) }
    LaunchedEffect(selector) {
        try {
            targets = TcpApi.devices().filter { target -> target.canStart &&
                ((selector.contactId != null && target.relation == "direct" && target.relationId == selector.contactId) ||
                (selector.deviceId != null && selector.publicKey != null && target.deviceId == selector.deviceId && target.publicKey == selector.publicKey)) }
        } catch (cancel: CancellationException) { throw cancel
        } catch (failure: Exception) { error = failure.message ?: "Gerätebeziehungen konnten nicht geladen werden."
        } finally { loading = false }
    }
    val choice = selected
    if (choice != null) {
        FamilyPicker(choice.screenTarget(), selector.name, onSelect = { target, entry ->
            onReplace(ShareDialog.FamilyView(target, entry, selector.name))
        }, onBack = { selected = null })
    } else {
        AlertDialog(onDismissRequest = onDismiss, title = { Text("Fensterzugang · ${selector.name}") }, text = {
            Column {
                if (loading) CircularProgressIndicator()
                error?.let { Text(it) }
                if (!loading && error == null && targets.isEmpty()) Text("Keine gespeicherte ausgehende Beziehung zu dieser Geräteidentität.")
                targets.forEach { target -> TextButton(onClick = { selected = target }) { Text("${target.relationLabel} · ${target.name}") } }
            }
        }, confirmButton = { TextButton(onClick = onDismiss) { Text("Schließen") } })
    }
}

/** The original receipt is never reinterpreted through PID, title or a new launch. */
@Composable
internal fun ConfirmedAppWindowDialog(confirmed: ConfirmedRemoteAppWindow, onDismiss: () -> Unit, onReplace: (ShareDialog) -> Unit) {
    var error by remember(confirmed) { mutableStateOf<String?>(null) }
    LaunchedEffect(confirmed) {
        try {
            val saved = confirmed.target
            val current = TcpApi.devices().singleOrNull { it.key == saved.key && it.deviceId == saved.deviceId && it.publicKey == saved.publicKey && it.canStart }
                ?: throw IllegalStateException("Die ursprüngliche Gerätebeziehung ist nicht mehr gültig.")
            check(current.nodeId.isBlank() || current.nodeId == confirmed.nodeId) { "Der Geräteschlüssel hat sich seit dem Appstart geändert." }
            val target = current.screenTarget(confirmed.nodeId).copy(control = true)
            val listing = FamilyApi.list(target)
            val entry = listing.entries.singleOrNull { it.launch_request == confirmed.request.requestId &&
                it.source == "window:${confirmed.window.windowId}" && it.identity.generation == confirmed.window.generation }
                ?: throw IllegalStateException("Der Host bestätigt kein eindeutiges aktuelles Fenster für diesen Startauftrag.")
            FamilyApi.validate(entry)
            onReplace(ShareDialog.FamilyView(target.copy(source = entry.source), entry, current.name))
        } catch (cancel: CancellationException) { throw cancel
        } catch (failure: Exception) { error = failure.message ?: "Der Fensterbeleg konnte nicht bestätigt werden." }
    }
    AlertDialog(onDismissRequest = onDismiss, title = { Text("Bestätigtes Appfenster") }, text = {
        Column { if (error == null) { CircularProgressIndicator(); Text("Geräteschlüssel und Fensterbeleg werden abgeglichen …") } else Text(error.orEmpty()) }
    }, confirmButton = { TextButton(onClick = onDismiss) { Text("Schließen") } })
}

private fun TcpTarget.screenTarget(node: String? = nodeId.takeIf { it.isNotBlank() }) =
    ScreenTarget(relation, relationId, deviceId, expectedNodeId = node)
