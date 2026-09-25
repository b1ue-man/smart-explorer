package app.smartexplorer.android.ui.analytics

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.AnalyzeChild
import app.smartexplorer.android.api.AnalyzeNode
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.EmptyState
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.kindIcon
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.more.TextReportDialog

/**
 * Storage analysis (spec F19): choose a place → scan with progress and [Abbrechen] → treemap and
 * the largest entries with bars; tap a folder → into it; back → up; ⋮ "In Dateien öffnen";
 * "n Pfade nicht lesbar [Bericht]".
 */
@Composable
internal fun AnalysisScreen(vm: AnalysisViewModel, onClose: () -> Unit) {
    // Composed after the tab's own handler, so it wins while there is a level to go up.
    BackHandler(enabled = vm.phase == ScanPhase.Result && vm.path.isNotEmpty()) { vm.up() }
    when (vm.phase) {
        ScanPhase.Setup -> ScanSetupPage(
            title = "Speicheranalyse",
            location = vm.location,
            onLocation = { vm.location = it },
            error = vm.error,
            startLabel = "Analysieren",
            onStart = { vm.start() },
            onClose = onClose,
            hint = "Lokale Speicher, Verbindungen und Share-Geräte lassen sich analysieren.",
        )
        ScanPhase.Scanning -> ScanProgressPage(
            title = "Speicheranalyse",
            location = vm.location,
            taskId = vm.taskId,
            onCancel = { vm.cancel() },
            onClose = onClose,
        )
        ScanPhase.Result -> ResultPage(vm, onClose)
    }
}

@Composable
private fun ResultPage(vm: AnalysisViewModel, onClose: () -> Unit) {
    var menu by remember { mutableStateOf(false) }
    var showReport by remember { mutableStateOf(false) }
    val node = vm.node
    SubPageScaffold(
        title = node?.name?.ifBlank { null } ?: "Speicheranalyse",
        onBack = { if (!vm.up()) onClose() },
        actions = {
            Box {
                IconButton(onClick = { menu = true }) { SeIcon(R.drawable.ic_more_vert, contentDescription = "Menü") }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    val location = node?.location
                    DropdownMenuItem(
                        text = { Text("In Dateien öffnen") },
                        enabled = location != null,
                        onClick = {
                            menu = false
                            location?.let { AppNav.send(NavRequest.OpenLocation(it)) }
                        },
                    )
                    DropdownMenuItem(text = { Text("Neu analysieren") }, onClick = {
                        menu = false
                        vm.start()
                    })
                    DropdownMenuItem(text = { Text("Anderer Ort") }, onClick = {
                        menu = false
                        vm.backToSetup()
                    })
                }
            }
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            LoadingBar(vm.loadingNode)
            when {
                node == null -> Unit
                node.children.isEmpty() -> EmptyState(R.drawable.ic_folder, "Dieser Ordner ist leer", message = Format.size(node.size))
                else -> NodeList(node, vm, onReport = { showReport = true })
            }
        }
    }
    val issues = vm.issues
    if (showReport && issues != null) {
        TextReportDialog("Leseprobleme", issues.text, onDismiss = { showReport = false })
    }
}

@Composable
private fun NodeList(node: AnalyzeNode, vm: AnalysisViewModel, onReport: () -> Unit) {
    val issues = vm.issues
    LazyColumn(Modifier.fillMaxSize()) {
        item(key = "header") {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                val where = if (vm.path.isEmpty()) vm.location.orEmpty() else vm.path.joinToString(" › ")
                Text(where, style = MaterialTheme.typography.bodySmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text(
                    "${Format.size(node.size)} · ${node.children.size} Einträge",
                    style = MaterialTheme.typography.titleSmall,
                )
                if (issues != null && issues.count > 0) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SeIcon(R.drawable.ic_warning, contentDescription = null, tint = MaterialTheme.colorScheme.error)
                        Text(
                            "${issues.count} Pfade nicht lesbar",
                            style = MaterialTheme.typography.bodyMedium,
                            modifier = Modifier.weight(1f).padding(start = 8.dp),
                        )
                        TextButton(onClick = onReport) { Text("Bericht") }
                    }
                }
                TreemapBlock(node.children, onOpen = { vm.open(it) })
            }
        }
        // Positional keys: a child named "header" or equal names (Google Drive) must not collide.
        items(node.children) { child -> ChildRow(child, node.size, onOpen = { vm.open(child) }) }
    }
}

@Composable
private fun ChildRow(child: AnalyzeChild, total: Long, onOpen: () -> Unit) {
    val share = shareOf(child.size, total)
    ListItem(
        headlineContent = { Text(child.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                val details = buildString {
                    append(Format.size(child.size)).append(" · ").append((share * 100).toInt()).append(" %")
                    if (child.isDir) append(" · ").append(child.childCount).append(" Einträge")
                }
                Text(details)
                LinearProgressIndicator(progress = { share }, modifier = Modifier.fillMaxWidth())
            }
        },
        leadingContent = {
            SeIcon(if (child.isDir) R.drawable.ic_folder else kindIcon(kindOf(tileKind(child))), contentDescription = null)
        },
        trailingContent = if (child.isDir) {
            { SeIcon(R.drawable.ic_chevron_right, contentDescription = null) }
        } else {
            null
        },
        modifier = Modifier.clickable(enabled = child.isDir, onClick = onOpen),
    )
}

/** `Entry.kind` values for the row icons (ui.common.kindIcon). */
private fun kindOf(kind: TileKind): String = when (kind) {
    TileKind.Folder -> "dir"
    TileKind.Image -> "image"
    TileKind.Video -> "video"
    TileKind.Audio -> "audio"
    TileKind.Archive -> "archive"
    TileKind.Document -> "document"
    TileKind.Other -> "other"
}
