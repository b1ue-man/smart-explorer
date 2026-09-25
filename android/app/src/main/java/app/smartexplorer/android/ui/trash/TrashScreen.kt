package app.smartexplorer.android.ui.trash

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.TrashItem
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.ConfirmDialog
import app.smartexplorer.android.ui.common.EmptyState
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.common.kindIcon
import kotlinx.coroutines.launch

/** Entries older than this many days are removed automatically (spec F11). */
internal const val TRASH_RETENTION_DAYS = 30

private sealed interface TrashDialog {
    data class DeleteForever(val ids: List<String>) : TrashDialog

    data object Empty : TrashDialog

    data class RestoreOccupied(val ids: List<String>, val occupied: Int) : TrashDialog
}

/**
 * App trash (spec F11): list (name, original place, deleted at, size); selection →
 * [Wiederherstellen] [Endgültig löschen]; menu [Papierkorb leeren]. Back leaves via [onBack].
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TrashScreen(onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    var items by remember { mutableStateOf<List<TrashItem>?>(null) }
    var loading by remember { mutableStateOf(true) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var selected by remember { mutableStateOf(setOf<String>()) }
    var dialog by remember { mutableStateOf<TrashDialog?>(null) }
    var menu by remember { mutableStateOf(false) }
    var reloadKey by remember { mutableIntStateOf(0) }

    LaunchedEffect(reloadKey) {
        loading = true
        if (reloadKey == 0) {
            try {
                FilesApi.trashPurge(TRASH_RETENTION_DAYS)
            } catch (e: CoreException) {
                Snackbars.show("Alte Einträge ließen sich nicht entfernen: ${e.message}")
            }
        }
        try {
            val list = FilesApi.trashList()
            items = list
            selected = selected.intersect(list.mapTo(HashSet()) { it.id })
            error = null
        } catch (e: CoreException) {
            error = e.message ?: e.kind
        } finally {
            loading = false
        }
    }

    /** Runs [block] with the busy indicator, reports errors, then reloads the list. */
    fun act(block: suspend () -> Unit) {
        scope.launch {
            busy = true
            try {
                block()
            } catch (e: CoreException) {
                Snackbars.show(e.message ?: e.kind)
            } finally {
                busy = false
                reloadKey++
            }
        }
    }

    fun restore(ids: List<String>) = act {
        val result = FilesApi.trashRestore(ids)
        selected = emptySet()
        Snackbars.show(
            "Wiederhergestellt: ${result.restored}" +
                if (result.renamed > 0) " (${result.renamed} mit neuem Namen, z. B. „Name (2)“)" else "",
        )
    }

    /** Restores directly, or asks first when some original places are occupied again. */
    fun requestRestore(ids: List<String>) {
        val byId = items.orEmpty().associateBy { it.id }
        scope.launch {
            busy = true
            val occupied = try {
                ids.count { id -> byId[id]?.let { exists(it.originalLocation) } == true }
            } catch (e: CoreException) {
                Snackbars.show("Ursprünglicher Ort nicht prüfbar: ${e.message ?: e.kind}")
                return@launch
            } finally {
                busy = false
            }
            if (occupied > 0) dialog = TrashDialog.RestoreOccupied(ids, occupied) else restore(ids)
        }
    }

    BackHandler(enabled = selected.isNotEmpty()) { selected = emptySet() }
    BackHandler(enabled = selected.isEmpty(), onBack = onBack)

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(if (selected.isEmpty()) "Papierkorb" else "${selected.size} ausgewählt") },
                navigationIcon = {
                    if (selected.isEmpty()) {
                        IconButton(onClick = onBack) { SeIcon(R.drawable.ic_arrow_back, contentDescription = "Zurück") }
                    } else {
                        IconButton(onClick = { selected = emptySet() }) {
                            SeIcon(R.drawable.ic_close, contentDescription = "Auswahl aufheben")
                        }
                    }
                },
                actions = {
                    val list = items.orEmpty()
                    if (selected.isNotEmpty() && selected.size < list.size) {
                        IconButton(onClick = { selected = list.mapTo(HashSet()) { it.id } }) {
                            SeIcon(R.drawable.ic_check, contentDescription = "Alle auswählen")
                        }
                    }
                    Box {
                        IconButton(onClick = { menu = true }) { SeIcon(R.drawable.ic_more_vert, contentDescription = "Menü") }
                        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                            DropdownMenuItem(
                                text = { Text("Papierkorb leeren") },
                                enabled = list.isNotEmpty(),
                                onClick = {
                                    menu = false
                                    dialog = TrashDialog.Empty
                                },
                            )
                        }
                    }
                },
            )
        },
        bottomBar = {
            if (selected.isNotEmpty()) {
                Surface(color = MaterialTheme.colorScheme.surfaceContainer) {
                    Row(
                        modifier = Modifier.fillMaxWidth().navigationBarsPadding().padding(16.dp),
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        OutlinedButton(
                            onClick = { dialog = TrashDialog.DeleteForever(selected.toList()) },
                            enabled = !busy,
                            modifier = Modifier.weight(1f),
                        ) { Text("Endgültig löschen") }
                        Button(
                            onClick = { requestRestore(selected.toList()) },
                            enabled = !busy,
                            modifier = Modifier.weight(1f),
                        ) { Text("Wiederherstellen") }
                    }
                }
            }
        },
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            LoadingBar(loading || busy)
            val list = items
            val message = error
            when {
                message != null -> ErrorCard(
                    message,
                    modifier = Modifier.padding(16.dp),
                    title = "Papierkorb nicht lesbar",
                    actionLabel = "Erneut",
                    onAction = { reloadKey++ },
                )
                list == null -> Unit
                list.isEmpty() -> EmptyState(
                    R.drawable.ic_trash,
                    "Papierkorb ist leer",
                    message = "Gelöschte Dateien bleiben hier $TRASH_RETENTION_DAYS Tage und lassen sich wiederherstellen.",
                )
                else -> LazyColumn(Modifier.fillMaxSize()) {
                    items(list, key = { it.id }) { item ->
                        TrashRow(
                            item = item,
                            selected = item.id in selected,
                            onToggle = { selected = if (item.id in selected) selected - item.id else selected + item.id },
                        )
                    }
                }
            }
        }
    }

    when (val current = dialog) {
        is TrashDialog.DeleteForever -> ConfirmDialog(
            title = "Endgültig löschen?",
            message = "${current.ids.size} Elemente werden endgültig gelöscht. Das lässt sich nicht rückgängig machen.",
            confirmLabel = "Löschen",
            destructive = true,
            onConfirm = {
                dialog = null
                act {
                    val task = FilesApi.awaitTask(FilesApi.trashDelete(current.ids))
                    selected = emptySet()
                    reportTask(task.state, task.message, "Endgültig gelöscht: ${current.ids.size}")
                }
            },
            onDismiss = { dialog = null },
        )
        TrashDialog.Empty -> ConfirmDialog(
            title = "Papierkorb leeren?",
            message = "Alle Einträge werden endgültig gelöscht. Das lässt sich nicht rückgängig machen.",
            confirmLabel = "Leeren",
            destructive = true,
            onConfirm = {
                dialog = null
                act {
                    val task = FilesApi.awaitTask(FilesApi.trashEmpty())
                    selected = emptySet()
                    reportTask(task.state, task.message, "Papierkorb geleert")
                }
            },
            onDismiss = { dialog = null },
        )
        is TrashDialog.RestoreOccupied -> ConfirmDialog(
            title = "Name belegt",
            message = "${current.occupied} von ${current.ids.size} Elementen haben am ursprünglichen Ort einen " +
                "gleichnamigen Eintrag. Beide behalten: das wiederhergestellte Element erhält einen Zusatz wie „Name (2)“.",
            confirmLabel = "Beide behalten",
            onConfirm = {
                dialog = null
                restore(current.ids)
            },
            onDismiss = { dialog = null },
        )
        null -> Unit
    }
}

@Composable
private fun TrashRow(item: TrashItem, selected: Boolean, onToggle: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(if (selected) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surface)
            .combinedClickable(onLongClick = onToggle, onClick = onToggle)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        SeIcon(
            if (selected) R.drawable.ic_check else kindIcon(if (item.isDir) "dir" else "other"),
            contentDescription = if (selected) "Ausgewählt" else null,
            tint = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Column(Modifier.weight(1f)) {
            Text(item.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                item.originalLocation,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                "Gelöscht ${Format.dateTime(item.deletedMs)}" + if (item.isDir) "" else " · ${Format.size(item.size)}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/** `true` when [location] exists; a missing entry (`not_found`) is `false`, other errors propagate. */
private suspend fun exists(location: String): Boolean = try {
    FilesApi.stat(location)
    true
} catch (e: CoreException) {
    if (e.kind == "not_found") false else throw e
}

private fun reportTask(state: String, message: String?, success: String) {
    when (state) {
        "done" -> Snackbars.show(success)
        "canceled" -> Snackbars.show("Abgebrochen")
        else -> Snackbars.show(message ?: "Vorgang fehlgeschlagen")
    }
}
