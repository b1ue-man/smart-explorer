package app.smartexplorer.android.ui.analytics

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedCard
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import app.smartexplorer.android.R
import app.smartexplorer.android.api.DuplicateGroup
import app.smartexplorer.android.api.DuplicateItem
import app.smartexplorer.android.ui.common.ConfirmDialog
import app.smartexplorer.android.ui.common.EmptyState
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.files.FilesLocation
import app.smartexplorer.android.ui.more.SubPageScaffold

/**
 * "Duplikate finden" (spec F20): place and minimum size → scan (progress, [Abbrechen]) → groups
 * (size, count) → [Kopien automatisch auswählen] (keeps the oldest) → [In den Papierkorb].
 */
@Composable
internal fun DuplicatesScreen(onBack: () -> Unit) {
    val vm = viewModel { DuplicatesViewModel() }
    LaunchedEffect(vm) { vm.preselect(FilesLocation.current.value) }
    when (vm.phase) {
        ScanPhase.Setup -> ScanSetupPage(
            title = "Duplikate finden",
            location = vm.location,
            onLocation = { vm.location = it },
            error = vm.error,
            startLabel = "Suchen",
            onStart = { vm.start() },
            onClose = onBack,
            hint = "Findet Dateien mit gleichem Inhalt. Remote-Orte ohne Papierkorb werden nur angezeigt.",
            options = { MinSizeChips(vm.minSize, onSelect = { vm.minSize = it }) },
        )
        ScanPhase.Scanning -> ScanProgressPage(
            title = "Duplikate finden",
            location = vm.location,
            taskId = vm.taskId,
            onCancel = { vm.cancel() },
            onClose = onBack,
        )
        ScanPhase.Result -> DuplicatesResult(vm, onBack)
    }
}

@Composable
private fun MinSizeChips(selected: Long, onSelect: (Long) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("Mindestgröße", style = MaterialTheme.typography.labelLarge)
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            DuplicatesViewModel.MIN_SIZES.forEach { size ->
                FilterChip(selected = selected == size, onClick = { onSelect(size) }, label = { Text("ab ${Format.size(size)}") })
            }
        }
    }
}

@Composable
private fun DuplicatesResult(vm: DuplicatesViewModel, onBack: () -> Unit) {
    var confirm by remember { mutableStateOf(false) }
    val groups = vm.groups
    SubPageScaffold(
        title = "Duplikate",
        onBack = onBack,
        actions = {
            IconButton(onClick = { vm.backToSetup() }) { SeIcon(R.drawable.ic_search, contentDescription = "Neue Suche") }
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            LoadingBar(vm.deleting)
            if (groups.isEmpty()) {
                EmptyState(
                    R.drawable.ic_duplicate,
                    "Keine Duplikate gefunden",
                    modifier = Modifier.weight(1f),
                    message = vm.location,
                    actionLabel = "Anderer Ort",
                    onAction = { vm.backToSetup() },
                )
            } else {
                GroupList(vm, groups, Modifier.weight(1f))
                if (!vm.trashUnsupported) {
                    HorizontalDivider()
                    Button(
                        onClick = { confirm = true },
                        enabled = vm.selected.isNotEmpty() && !vm.deleting,
                        modifier = Modifier.fillMaxWidth().padding(16.dp),
                    ) { Text("In den Papierkorb (${vm.selected.size})") }
                }
            }
        }
    }
    if (confirm) {
        ConfirmDialog(
            title = "${vm.selected.size} Dateien in den Papierkorb?",
            message = "Die gewählten Kopien kommen in den Papierkorb und lassen sich von dort wiederherstellen.",
            confirmLabel = "In den Papierkorb",
            destructive = true,
            onConfirm = {
                confirm = false
                vm.trashSelected()
            },
            onDismiss = { confirm = false },
        )
    }
}

@Composable
private fun GroupList(vm: DuplicatesViewModel, groups: List<DuplicateGroup>, modifier: Modifier = Modifier) {
    val copies = groups.sumOf { it.items.size - 1 }
    val wasted = groups.sumOf { it.size * (it.items.size - 1) }
    LazyColumn(modifier.fillMaxWidth()) {
        item(key = "summary") {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    "${groups.size} Gruppen · $copies Kopien · ${Format.size(wasted)} durch Kopien belegt",
                    style = MaterialTheme.typography.titleSmall,
                )
                if (vm.trashUnsupported) {
                    ErrorCard("Dieser Ort hat keinen Papierkorb – die Duplikate werden nur angezeigt.", title = "Nur Anzeige")
                } else {
                    OutlinedButton(onClick = { vm.autoSelect() }) { Text("Kopien automatisch auswählen") }
                }
            }
        }
        // Positional keys: equal Google Drive names share one location (B2), so locations may repeat.
        items(groups) { group ->
            GroupCard(group, vm.selected, enabled = !vm.trashUnsupported, onToggle = { vm.toggle(group, it) })
        }
    }
}

@Composable
private fun GroupCard(group: DuplicateGroup, selected: Set<String>, enabled: Boolean, onToggle: (String) -> Unit) {
    OutlinedCard(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
        Text(
            "${Format.size(group.size)} · ${group.items.size} Kopien",
            style = MaterialTheme.typography.titleSmall,
            modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp),
        )
        group.items.forEach { item -> CopyRow(item, item.location in selected, enabled, onToggle) }
    }
}

@Composable
private fun CopyRow(item: DuplicateItem, checked: Boolean, enabled: Boolean, onToggle: (String) -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .toggleable(value = checked, enabled = enabled, role = Role.Checkbox, onValueChange = { onToggle(item.location) })
            .padding(horizontal = 8.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = checked, onCheckedChange = null, enabled = enabled)
        Column(Modifier.weight(1f)) {
            Text(item.location, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text("Geändert ${Format.dateTime(item.mtimeMs)}", style = MaterialTheme.typography.bodySmall)
        }
    }
}
