package app.smartexplorer.android.ui.analytics

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.AnalyzeKind
import app.smartexplorer.android.api.AnalyzeNode
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.EmptyState
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.more.TextReportDialog

/** Which report dialog the result page shows. */
private enum class Report { Issues, Protected }

/**
 * Storage analysis (spec F19, B1–B3): choose a place → scan with progress and [Abbrechen] →
 * treemap and the largest entries with bars; tap a folder → into it; back → up; ⋮ "In Dateien
 * öffnen"; "n Pfade nicht lesbar [Bericht]"; areas Android locks are counted apart, sized by the
 * platform where possible, with the usage access card for the other apps' folders.
 */
@Composable
internal fun AnalysisScreen(vm: AnalysisViewModel, onClose: () -> Unit) {
    val context = LocalContext.current
    LaunchedEffect(vm, context) { vm.attach(context) }
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
            cancelEnabled = true,
        )
        ScanPhase.Result -> ResultPage(vm, onClose)
    }
}

@Composable
private fun ResultPage(vm: AnalysisViewModel, onClose: () -> Unit) {
    var menu by remember { mutableStateOf(false) }
    var report by remember { mutableStateOf<Report?>(null) }
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
                node.children.isEmpty() -> EmptyFolder(node, vm)
                else -> NodeList(node, vm, onReport = { report = it })
            }
        }
    }
    val issues = vm.issues
    when {
        issues == null -> Unit
        report == Report.Issues -> TextReportDialog("Leseprobleme", issues.text, onDismiss = { report = null })
        report == Report.Protected -> TextReportDialog(
            "Geschützte Bereiche",
            issues.protectedText.ifBlank { PROTECTED_EXPLANATION },
            onDismiss = { report = null },
        )
    }
}

/** Shows the usage access card where the other apps' folders matter: the root and locked folders. */
private fun showsUsageCard(vm: AnalysisViewModel): Boolean =
    vm.appDataInTree && !vm.usageHintDismissed && (vm.path.isEmpty() || vm.insideProtected)

/** An empty folder; a locked one (or a locked analysed root) says why it is empty. */
@Composable
private fun EmptyFolder(node: AnalyzeNode, vm: AnalysisViewModel) {
    val lockedRoot = vm.path.isEmpty() && (vm.issues?.protectedCount ?: 0L) > 0L
    if (!vm.insideProtected && !lockedRoot) {
        EmptyState(R.drawable.ic_folder, "Dieser Ordner ist leer", message = Format.size(node.size))
        return
    }
    Column(Modifier.fillMaxSize()) {
        if (showsUsageCard(vm)) {
            UsageAccessCard(
                appDataIncluded = vm.appDataIncluded,
                onReanalyze = { vm.start() },
                onDismiss = { vm.dismissUsageHint() },
                modifier = Modifier.padding(16.dp),
            )
        }
        EmptyState(R.drawable.ic_lock, "Von Android geschützt", modifier = Modifier.weight(1f), message = PROTECTED_EXPLANATION)
    }
}

@Composable
private fun NodeList(node: AnalyzeNode, vm: AnalysisViewModel, onReport: (Report) -> Unit) {
    val issues = vm.issues
    // Display-only estimates may or may not be part of node.size; the larger sum is the whole.
    val total = remember(node) { maxOf(node.size, node.children.sumOf { it.size }) }
    val estimated = remember(node) { node.children.filter { it.rowKind == AnalyzeKind.Rest }.sumOf { it.size } }
    val entries = remember(node) { node.children.count { it.rowKind != AnalyzeKind.Rest } }
    LazyColumn(Modifier.fillMaxSize()) {
        item(key = "header") {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                val where = if (vm.path.isEmpty()) vm.location.orEmpty() else vm.path.joinToString(" › ")
                Text(where, style = MaterialTheme.typography.bodySmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text("${Format.size(total)} · $entries Einträge", style = MaterialTheme.typography.titleSmall)
                if (estimated > 0) {
                    Text(
                        "davon ≈ ${Format.size(estimated)} laut Android (nicht als Dateien erfasst)",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (issues != null && issues.count > 0) {
                    NoticeRow(
                        R.drawable.ic_warning,
                        "${issues.count} Pfade nicht lesbar",
                        tint = MaterialTheme.colorScheme.error,
                        actionLabel = "Bericht",
                        onAction = { onReport(Report.Issues) },
                    )
                }
                // Android may also just hide other apps' folders (count 0, text set).
                val protectedShown = issues != null && (issues.protectedCount > 0 || issues.protectedText.isNotBlank())
                if (issues != null && protectedShown && (vm.path.isEmpty() || vm.insideProtected)) {
                    NoticeRow(
                        R.drawable.ic_lock,
                        if (issues.protectedCount > 0) protectedLabel(issues.protectedCount) else "Fremde App-Ordner von Android ausgeblendet",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        actionLabel = "Details",
                        onAction = { onReport(Report.Protected) },
                    )
                }
                if (showsUsageCard(vm)) {
                    UsageAccessCard(
                        appDataIncluded = vm.appDataIncluded,
                        onReanalyze = { vm.start() },
                        onDismiss = { vm.dismissUsageHint() },
                    )
                }
                TreemapBlock(node.children, onOpen = { vm.open(it) })
            }
        }
        // Positional keys: a child named "header" or equal names (Google Drive) must not collide.
        items(node.children) { child -> ChildRow(child, total, onOpen = { vm.open(child) }) }
    }
}
