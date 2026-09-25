package app.smartexplorer.android.ui.connections

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import app.smartexplorer.android.R
import app.smartexplorer.android.api.Connection
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.ConfirmDialog
import app.smartexplorer.android.ui.common.EmptyState
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.more.SectionHeader
import app.smartexplorer.android.ui.more.SubPageScaffold

/** Confirmations of the row menu. */
private sealed interface ConnConfirm {
    val connection: Connection

    data class Delete(override val connection: Connection) : ConnConfirm

    data class ForgetKey(override val connection: Connection) : ConnConfirm
}

/**
 * "Mehr → Verbindungen" (spec F12, F13): saved SFTP/FTP/FTPS/WebDAV connections with ⋮ (Öffnen,
 * Bearbeiten, Testen, Hostschlüssel vergessen, Löschen), (+) for a new one and Google Drive.
 * [startWithNewForm] opens the form for a new connection ("Verbindung hinzufügen").
 */
@Composable
fun ConnectionsScreen(startWithNewForm: Boolean, onNewFormShown: () -> Unit, onBack: () -> Unit) {
    val vm = viewModel { ConnectionsViewModel() }
    var confirm by remember { mutableStateOf<ConnConfirm?>(null) }
    LaunchedEffect(Unit) { vm.load() }
    LaunchedEffect(startWithNewForm) {
        if (startWithNewForm) {
            vm.openNew()
            onNewFormShown()
        }
    }

    val form = vm.form
    if (form != null) {
        BackHandler { vm.closeForm() }
        ConnectionFormPage(
            state = form,
            onTest = { vm.testForm(form) },
            onSave = { vm.saveForm(form) },
            onClose = { vm.closeForm() },
        )
    } else {
        SubPageScaffold(
            title = "Verbindungen",
            onBack = onBack,
            floatingActionButton = {
                FloatingActionButton(onClick = { vm.openNew() }) { SeIcon(R.drawable.ic_add, contentDescription = "Verbindung hinzufügen") }
            },
        ) { padding ->
            Column(Modifier.fillMaxSize().padding(padding)) {
                LoadingBar(vm.loading)
                ConnectionList(
                    vm,
                    onAsk = { confirm = it },
                    modifier = Modifier.weight(1f),
                )
            }
        }
    }

    when (val pending = confirm) {
        is ConnConfirm.Delete -> ConfirmDialog(
            title = "Verbindung löschen?",
            message = "„${pending.connection.label}“ wird gelöscht. Favoriten und Tabs dieser Verbindung werden " +
                "entfernt; Sync-Jobs, die sie nutzen, werden nur gemeldet.",
            confirmLabel = "Löschen",
            destructive = true,
            onConfirm = {
                confirm = null
                vm.delete(pending.connection)
            },
            onDismiss = { confirm = null },
        )
        is ConnConfirm.ForgetKey -> ConfirmDialog(
            title = "Hostschlüssel vergessen?",
            message = "Nur tun, wenn der Server seinen Schlüssel erwartet geändert hat. Beim nächsten Verbinden " +
                "wird der dann angebotene Schlüssel ohne Rückfrage gespeichert.",
            confirmLabel = "Vergessen",
            destructive = true,
            onConfirm = {
                confirm = null
                vm.forgetHostKey(pending.connection)
            },
            onDismiss = { confirm = null },
        )
        null -> Unit
    }
    vm.report?.let { RemovalReportDialog(it, onDismiss = { vm.report = null }) }
}

@Composable
private fun ConnectionList(vm: ConnectionsViewModel, onAsk: (ConnConfirm) -> Unit, modifier: Modifier = Modifier) {
    val error = vm.loadError
    val list = vm.connections
    LazyColumn(modifier.fillMaxSize()) {
        if (error != null) {
            item(key = "error") {
                ErrorCard(
                    error,
                    modifier = Modifier.padding(16.dp),
                    title = "Verbindungen nicht geladen",
                    actionLabel = "Erneut",
                    onAction = { vm.load() },
                )
            }
        }
        if (list != null && list.isEmpty()) {
            item(key = "empty") {
                EmptyState(
                    R.drawable.ic_cloud,
                    "Noch keine Verbindungen",
                    modifier = Modifier.padding(vertical = 8.dp),
                    message = "SFTP-, FTP- oder WebDAV-Server hinzufügen, um sie wie Ordner zu durchsuchen.",
                    actionLabel = "Verbindung hinzufügen",
                    onAction = { vm.openNew() },
                )
            }
        }
        items(list.orEmpty(), key = { it.id }) { connection ->
            ConnectionRow(connection, vm, onAsk)
        }
        item(key = "gdrive") {
            // Several children of one lazy item would overlap: stack them explicitly.
            Column(Modifier.padding(bottom = 88.dp)) {
                SectionHeader("Google Drive")
                GoogleDriveSection(Modifier.padding(horizontal = 16.dp))
            }
        }
    }
}

@Composable
private fun ConnectionRow(connection: Connection, vm: ConnectionsViewModel, onAsk: (ConnConfirm) -> Unit) {
    var menu by remember { mutableStateOf(false) }
    val address = buildString {
        append(ConnectionDraft.protocolLabel(connection.protocol)).append(" · ")
        if (connection.user.isNotBlank()) append(connection.user).append('@')
        append(connection.host)
        if (connection.port > 0) append(':').append(connection.port)
    }
    ListItem(
        headlineContent = { Text(connection.label.ifBlank { connection.host }, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(address, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        leadingContent = { SeIcon(R.drawable.ic_cloud, contentDescription = null) },
        trailingContent = {
            Box {
                IconButton(onClick = { menu = true }) { SeIcon(R.drawable.ic_more_vert, contentDescription = "Aktionen") }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    DropdownMenuItem(text = { Text("Öffnen") }, onClick = {
                        menu = false
                        openInFiles(connection)
                    })
                    DropdownMenuItem(text = { Text("Bearbeiten") }, onClick = {
                        menu = false
                        vm.openEdit(connection)
                    })
                    DropdownMenuItem(text = { Text("Testen") }, onClick = {
                        menu = false
                        vm.test(connection)
                    })
                    if (connection.protocol == "sftp") {
                        DropdownMenuItem(text = { Text("Hostschlüssel vergessen") }, onClick = {
                            menu = false
                            onAsk(ConnConfirm.ForgetKey(connection))
                        })
                    }
                    DropdownMenuItem(text = { Text("Löschen") }, onClick = {
                        menu = false
                        onAsk(ConnConfirm.Delete(connection))
                    })
                }
            }
        },
        modifier = Modifier.clickable { openInFiles(connection) },
    )
}

private fun openInFiles(connection: Connection) {
    if (connection.location.isNotBlank()) AppNav.send(NavRequest.OpenLocation(connection.location))
}

/** After removing a connection, device or room: removed favorites and still affected sync jobs. */
@Composable
internal fun RemovalReportDialog(report: RemovalReport, onDismiss: () -> Unit) {
    val removal = report.removal
    val text = buildString {
        append("„${report.name}“ wurde entfernt.")
        if (removal.removedFavorites > 0) append("\n${removal.removedFavorites} Favoriten entfernt.")
        if (removal.orphanedJobs.isNotEmpty()) {
            append("\n\nDiese Sync-Jobs nutzen den Ort noch und laufen nicht mehr, bis sie angepasst sind:\n")
            removal.orphanedJobs.forEach { append("• ").append(it).append('\n') }
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Entfernt") },
        text = { Text(text.trimEnd()) },
        confirmButton = { TextButton(onClick = onDismiss) { Text("OK") } },
    )
}
