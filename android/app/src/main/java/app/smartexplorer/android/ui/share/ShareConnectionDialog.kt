package app.smartexplorer.android.ui.share

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.SHARE_DIRECT
import app.smartexplorer.android.api.SHARE_READ_ONLY
import app.smartexplorer.android.api.SHARE_READ_WRITE
import app.smartexplorer.android.api.ShareConnectionExport
import app.smartexplorer.android.api.ShareConnections
import app.smartexplorer.android.api.SharePolicyApi
import app.smartexplorer.android.core.CoreException

private data class ConnectionChoice(val account: String, val label: String, val access: String?, val saved: Boolean)

@Composable
internal fun ShareConnectionDialog(scope: String, vm: ShareViewModel, onDismiss: () -> Unit) {
    var answer by remember { mutableStateOf<ShareConnections?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var loading by remember { mutableStateOf(true) }
    var reload by remember { mutableIntStateOf(0) }
    var chosen by remember { mutableStateOf<ConnectionChoice?>(null) }
    LaunchedEffect(scope, reload) {
        loading = true
        try {
            answer = SharePolicyApi.connections(scope)
            error = null
        } catch (e: CoreException) {
            error = "Verbindungen nicht geladen: ${e.message ?: e.kind}"
        } finally {
            loading = false
        }
    }
    // On a Saved-store failure the existing snapshot still permits withdrawal, never a new grant.
    val current = answer?.sharedConnections ?: if (scope == SHARE_DIRECT) {
        vm.status?.connectionExports?.direct.orEmpty()
    } else {
        vm.status?.connectionExports?.rooms?.get(scope).orEmpty()
    }
    val choices = connectionChoices(if (error == null) answer else null, current)
    AlertDialog(
        onDismissRequest = { if (chosen == null) onDismiss() },
        title = { Text("Gespeicherte Verbindungen freigeben") },
        text = {
            Column {
                Text(answer?.warning?.takeIf { it.isNotBlank() } ?: "Freigegebene Verbindungen nutzen deine gespeicherten Zugangsdaten. Wähle jedes Konto und dessen Rechte einzeln.")
                if (loading) LinearProgressIndicator()
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                if (!loading && error == null && choices.isEmpty()) Text("Keine gespeicherten Verbindungen.")
                LazyColumn(Modifier.heightIn(max = 360.dp)) {
                    items(choices, key = { it.account }) { choice ->
                        ListItem(
                            headlineContent = { Text(choice.label.ifBlank { choice.account }) },
                            supportingContent = {
                                Text((choice.access?.let(::accessLabel) ?: "Nicht freigegeben") + if (!choice.saved) " · nicht mehr gespeichert" else "")
                            },
                            modifier = Modifier.clickable(enabled = !loading) { chosen = choice },
                        )
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = { reload++ }, enabled = !loading && chosen == null) { Text("Erneut laden") } },
        dismissButton = { TextButton(onClick = onDismiss, enabled = chosen == null) { Text("Schließen") } },
    )
    chosen?.let { selection ->
        key(selection.account) {
            ConnectionAccessDialog(scope, selection, vm, onDismiss = { chosen = null }, onSaved = {
                chosen = null
                reload++
            })
        }
    }
}

private fun connectionChoices(answer: ShareConnections?, shared: List<ShareConnectionExport>): List<ConnectionChoice> {
    val result = linkedMapOf<String, ConnectionChoice>()
    answer?.connections?.forEach { connection ->
        result[connection.account] = ConnectionChoice(connection.account, connection.label, connection.access, true)
    }
    shared.forEach { connection ->
        if (connection.account !in result) result[connection.account] = ConnectionChoice(connection.account, connection.account, connection.access, false)
    }
    return result.values.toList()
}

@Composable
private fun ConnectionAccessDialog(scope: String, choice: ConnectionChoice, vm: ShareViewModel, onDismiss: () -> Unit, onSaved: () -> Unit) {
    val action = rememberShareAction()
    var shared by remember { mutableStateOf(choice.access != null) }
    var access by remember { mutableStateOf(choice.access ?: SHARE_READ_ONLY) }
    ShareActionDialog(
        title = choice.label.ifBlank { choice.account },
        state = action,
        onDismiss = onDismiss,
        enabled = (shared != (choice.access != null) || (shared && access != choice.access)) &&
            (!shared || (choice.saved && access in setOf(SHARE_READ_ONLY, SHARE_READ_WRITE))),
        onConfirm = {
            val selected = shared
            val rights = if (selected) access else null
            val expectedShared = true.takeIf { selected && choice.access != null }
            action.submit(vm, "Verbindungsfreigabe nicht gespeichert", onSaved) {
                SharePolicyApi.setConnectionExport(scope, choice.account, selected, rights, expectedShared)
            }
        },
    ) {
        SelectionContainer { Text(choice.account) }
        Text("Andere zugelassene Geräte benutzen diese Verbindung mit deinen gespeicherten Zugangsdaten. Providerrechte und Kontakt-/Raumrechte gelten zusätzlich.")
        ShareChoice("Dieses Konto freigeben", shared, !action.saving && (choice.saved || shared)) { shared = it }
        if (shared) ShareAccessPicker(access, !action.saving && choice.saved) { access = it }
        if (!choice.saved) Text("Das Konto ist nicht mehr gespeichert. Diese Freigabe kann weiterhin entzogen werden.")
    }
}
