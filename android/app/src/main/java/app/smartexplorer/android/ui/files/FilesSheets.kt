package app.smartexplorer.android.ui.files

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.R
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.PropertiesResult
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.SeIcon

/** Tab list (spec F3): place and path per tab, ✕ per tab, "+ Neuer Tab"; tap switches. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TabsSheet(vm: FilesViewModel, onDismiss: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Text("Tabs", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp))
        val activeId = vm.paneTabs.getOrNull(vm.activePane)
        val canClose = vm.tabs.size > vm.paneCount
        LazyColumn(Modifier.fillMaxWidth()) {
            items(vm.tabs.toList(), key = { it.id }) { tab ->
                val pane = vm.paneTabs.indexOf(tab.id).takeIf { it >= 0 && it < vm.paneCount }
                ListItem(
                    headlineContent = {
                        Text(tab.listing?.title ?: tab.location, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    },
                    supportingContent = {
                        val where = if (pane != null && vm.paneCount > 1) " · Bereich ${pane + 1}" else ""
                        Text((tab.listing?.location ?: tab.location) + where, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    },
                    leadingContent = { SeIcon(R.drawable.ic_tab, contentDescription = null) },
                    trailingContent = if (canClose) {
                        {
                            IconButton(onClick = { vm.closeTab(tab.id) }) {
                                SeIcon(R.drawable.ic_close, contentDescription = "Tab schließen")
                            }
                        }
                    } else {
                        null
                    },
                    colors = ListItemDefaults.colors(
                        containerColor = if (tab.id == activeId) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent,
                    ),
                    modifier = Modifier.clickable {
                        vm.selectTab(tab.id)
                        onDismiss()
                    },
                )
            }
            item(key = "new") {
                ListItem(
                    headlineContent = { Text("Neuer Tab") },
                    leadingContent = { SeIcon(R.drawable.ic_add, contentDescription = null) },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    modifier = Modifier.clickable {
                        vm.newTab(null)
                        onDismiss()
                    },
                )
            }
        }
    }
}

private val SORT_KEYS = listOf("name" to "Name", "size" to "Größe", "mtime" to "Datum", "type" to "Typ")

/** "Ansicht": sort key and direction, folders first, hidden files, compact rows (spec F4). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ViewOptionsSheet(sortKey: String, sortDesc: Boolean, onSort: (String, Boolean) -> Unit, onDismiss: () -> Unit) {
    val dirsFirst by AppPrefs.dirsFirst.collectAsStateWithLifecycle()
    val showHidden by AppPrefs.showHidden.collectAsStateWithLifecycle()
    val compact by AppPrefs.compact.collectAsStateWithLifecycle()
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(bottom = 24.dp)) {
            Text("Ansicht", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(bottom = 8.dp))
            Text("Sortieren nach", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
            SORT_KEYS.forEach { (key, label) ->
                Row(
                    modifier = Modifier.fillMaxWidth().selectable(selected = sortKey == key, onClick = { onSort(key, sortDesc) }, role = Role.RadioButton),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RadioButton(selected = sortKey == key, onClick = null)
                    Text(label, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.padding(start = 8.dp))
                }
            }
            SwitchRow("Absteigend", sortDesc) { onSort(sortKey, it) }
            SwitchRow("Ordner zuerst", dirsFirst) { AppPrefs.setDirsFirst(it) }
            SwitchRow("Versteckte Dateien zeigen", showHidden) { AppPrefs.setShowHidden(it) }
            SwitchRow("Kompakte Zeilen", compact) { AppPrefs.setCompact(it) }
        }
    }
}

@Composable
private fun SwitchRow(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth().toggleable(value = checked, onValueChange = onChange, role = Role.Switch).padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = null)
    }
}

/**
 * Properties (spec F7): name, place, type, size (folders recursively, with progress), count,
 * modified, created; [Pfad kopieren].
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun PropertiesSheet(state: PropertiesState, tasks: List<TaskInfo>, onCopyPath: () -> Unit, onDismiss: () -> Unit) {
    val task = tasks.firstOrNull { it.id == state.taskId }
    val decoded = decodeProperties(task)
    val result = decoded.first
    val error = state.error ?: decoded.second ?: task?.takeIf { it.state == "failed" }?.let { it.message ?: "Berechnung fehlgeschlagen" }
    val running = error == null && (state.taskId == null || task == null || task.isActive)
    val single = state.entries.singleOrNull()
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(state.title, style = MaterialTheme.typography.titleLarge, maxLines = 2, overflow = TextOverflow.Ellipsis)
            if (running) LinearProgressIndicator(Modifier.fillMaxWidth())
            Property("Ort", result?.location ?: single?.location ?: state.locations.joinToString("\n"))
            Property("Typ", single?.let { typeLabel(it) } ?: "Mehrere Elemente")
            val size = when {
                result != null -> Format.size(result.bytes)
                task != null && task.doneBytes > 0 -> "${Format.size(task.doneBytes)} …"
                single != null && !single.isDir -> Format.size(single.size)
                else -> "…"
            }
            Property("Größe", size)
            if (result != null) {
                Property("Anzahl", "${result.files} Dateien, ${result.dirs} Ordner")
            } else if (task != null && task.doneItems > 0) {
                Property("Anzahl", "${task.doneItems} …")
            }
            Property("Geändert", Format.dateTime(result?.mtimeMs ?: single?.mtimeMs ?: 0))
            Property("Erstellt", Format.dateTime(result?.btimeMs ?: 0))
            if (error != null) {
                Text(error, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            }
            TextButton(onClick = onCopyPath, modifier = Modifier.align(Alignment.End)) { Text("Pfad kopieren") }
        }
    }
}

@Composable
private fun Property(label: String, value: String) {
    Row(Modifier.fillMaxWidth()) {
        Text(
            label,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.width(96.dp),
        )
        Text(value, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
    }
}

/** Result of a finished properties task, or the reason it cannot be read. */
private fun decodeProperties(task: TaskInfo?): Pair<PropertiesResult?, String?> {
    if (task == null || task.state != "done") return null to null
    return try {
        FilesApi.resultOf(task, PropertiesResult.serializer()) to null
    } catch (e: CoreException) {
        null to (e.message ?: e.kind)
    }
}

private fun typeLabel(entry: Entry): String {
    if (entry.isDir) return "Ordner"
    val kind = when (entry.kind) {
        "image" -> "Bild"
        "video" -> "Video"
        "audio" -> "Audio"
        "text" -> "Text"
        "archive" -> "Archiv"
        "document" -> "Dokument"
        "apk" -> "App-Paket"
        else -> "Datei"
    }
    val ext = entry.ext.removePrefix(".").lowercase()
    return if (ext.isEmpty()) kind else "$kind (.$ext)"
}
