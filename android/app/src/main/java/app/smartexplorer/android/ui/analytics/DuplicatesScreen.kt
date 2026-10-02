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
import app.smartexplorer.android.api.ReclaimSummary
import app.smartexplorer.android.ui.common.ConfirmDialog
import app.smartexplorer.android.ui.common.EmptyState
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.files.FilesLocation
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.more.TextReportDialog

/** A report dialog: title and text. */
private class ReportText(val title: String, val text: String)

/** One line of the group list: a group's header or one of its copies (lazy even for huge groups). */
private sealed interface GroupLine {
    class Header(val group: DuplicateGroup, val first: Boolean) : GroupLine
    class Copy(val group: DuplicateGroup, val item: DuplicateItem) : GroupLine
}

/**
 * "Duplikate finden" (spec F20, B5): place and minimum size → scan (phases, [Abbrechen]) → all
 * groups (size, count) with the search totals, an early stop, unreadable paths and protected
 * areas → [Kopien automatisch auswählen] (keeps the oldest) → [In den Papierkorb].
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
            hint = "Vergleicht alle Dateien ab der Mindestgröße und findet die mit gleichem Inhalt; gelesen werden nur " +
                "Dateien, deren Größe mehrfach vorkommt. Remote-Orte ohne Papierkorb werden nur angezeigt.",
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
    var report by remember { mutableStateOf<ReportText?>(null) }
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
                NoGroups(vm, onReport = { report = it }, modifier = Modifier.weight(1f))
            } else {
                GroupList(vm, groups, onReport = { report = it }, modifier = Modifier.weight(1f))
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
    report?.let { TextReportDialog(it.title, it.text, onDismiss = { report = null }) }
}

/**
 * "1.234 Dateien durchsucht (12 GB) · 567 ab 1 MB, 120 gleich große verglichen"; `null` without
 * totals. Files of a size no other file has cannot have a copy and are not read.
 */
private fun searchFacts(summary: ReclaimSummary?, minSize: Long): String? = summary?.let {
    "${it.files} Dateien durchsucht (${Format.size(it.bytes)}) · ${it.candidates} ab ${Format.size(minSize)}, " +
        "${it.compared} gleich große verglichen"
}

/** Early stop, unreadable paths and protected areas of the search (B1/B5); nothing when complete. */
@Composable
private fun SearchNotices(summary: ReclaimSummary?, onReport: (ReportText) -> Unit) {
    if (summary == null) return
    // The core words every reason itself (walk stopped early, candidates not compared, groups not
    // shown), one per line.
    summary.limit?.trim()?.takeIf { it.isNotEmpty() }?.let { limit ->
        ErrorCard(limit, title = "Ergebnis unvollständig")
    }
    if (summary.errorCount > 0) {
        NoticeRow(
            R.drawable.ic_warning,
            "${summary.errorCount} Pfade nicht lesbar",
            tint = MaterialTheme.colorScheme.error,
            actionLabel = "Bericht",
            onAction = { onReport(ReportText("Leseprobleme", summary.errorText)) },
        )
    }
    if (summary.protectedCount > 0) {
        NoticeRow(
            R.drawable.ic_lock,
            protectedLabel(summary.protectedCount),
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            actionLabel = "Details",
            onAction = { onReport(ReportText("Geschützte Bereiche", PROTECTED_EXPLANATION)) },
        )
    }
}

/** No groups: the search's notices, then why the list is empty. */
@Composable
private fun NoGroups(vm: DuplicatesViewModel, onReport: (ReportText) -> Unit, modifier: Modifier = Modifier) {
    val summary = vm.summary
    val locked = summary != null && summary.protectedCount > 0 && summary.files == 0L
    Column(modifier.fillMaxSize()) {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            SearchNotices(summary, onReport)
        }
        EmptyState(
            if (locked) R.drawable.ic_lock else R.drawable.ic_duplicate,
            if (locked) "Von Android geschützt" else "Keine Duplikate gefunden",
            modifier = Modifier.weight(1f),
            message = if (locked) PROTECTED_EXPLANATION else searchFacts(summary, vm.searchedMinSize) ?: vm.location,
            actionLabel = "Anderer Ort",
            onAction = { vm.backToSetup() },
        )
    }
}

@Composable
private fun GroupList(
    vm: DuplicatesViewModel,
    groups: List<DuplicateGroup>,
    onReport: (ReportText) -> Unit,
    modifier: Modifier = Modifier,
) {
    val copies = remember(groups) { groups.sumOf { it.items.size - 1 } }
    val wasted = remember(groups) { groups.sumOf { it.size * (it.items.size - 1) } }
    val lines = remember(groups) { linesOf(groups) }
    LazyColumn(modifier.fillMaxWidth()) {
        item(key = "summary") {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    "${groups.size} Gruppen · $copies Kopien · ${Format.size(wasted)} durch Kopien belegt",
                    style = MaterialTheme.typography.titleSmall,
                )
                searchFacts(vm.summary, vm.searchedMinSize)?.let {
                    Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                SearchNotices(vm.summary, onReport)
                if (vm.trashUnsupported) {
                    ErrorCard("Dieser Ort hat keinen Papierkorb – die Duplikate werden nur angezeigt.", title = "Nur Anzeige")
                } else {
                    OutlinedButton(onClick = { vm.autoSelect() }) { Text("Kopien automatisch auswählen") }
                }
            }
        }
        // Positional keys: equal Google Drive names share one location (B2), so locations may repeat.
        items(lines, contentType = { if (it is GroupLine.Header) "header" else "copy" }) { line ->
            when (line) {
                is GroupLine.Header -> GroupHeader(line.group, line.first)
                is GroupLine.Copy -> {
                    val location = line.item.location
                    val shared = location in vm.ambiguous
                    CopyRow(
                        line.item,
                        checked = location in vm.selected,
                        enabled = !vm.trashUnsupported && !shared,
                        shared = shared,
                        onToggle = { vm.toggle(line.group, it) },
                    )
                }
            }
        }
    }
}

private fun linesOf(groups: List<DuplicateGroup>): List<GroupLine> = buildList {
    groups.forEachIndexed { index, group ->
        add(GroupLine.Header(group, first = index == 0))
        group.items.forEach { add(GroupLine.Copy(group, it)) }
    }
}

@Composable
private fun GroupHeader(group: DuplicateGroup, first: Boolean) {
    Column(Modifier.fillMaxWidth()) {
        if (!first) HorizontalDivider(Modifier.padding(horizontal = 16.dp))
        Text(
            "${Format.size(group.size)} · ${group.items.size} Kopien",
            style = MaterialTheme.typography.titleSmall,
            modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp, bottom = 4.dp),
        )
    }
}

@Composable
private fun CopyRow(item: DuplicateItem, checked: Boolean, enabled: Boolean, shared: Boolean, onToggle: (String) -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .toggleable(value = checked, enabled = enabled, role = Role.Checkbox, onValueChange = { onToggle(item.location) })
            .padding(horizontal = 16.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = checked, onCheckedChange = null, enabled = enabled)
        Column(Modifier.weight(1f).padding(start = 8.dp)) {
            Text(item.location, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text("Geändert ${Format.dateTime(item.mtimeMs)}", style = MaterialTheme.typography.bodySmall)
            if (shared) {
                Text(
                    "Gleicher Name am selben Ort – nicht einzeln löschbar",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}
