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
private enum class Report { Issues, Protected, Notes }

/** Notes shown in the header before "Alle Hinweise" collects the rest. */
private const val INLINE_NOTES = 2

/**
 * Storage analysis (spec F19, B1–B3, B6): choose a place → scan with progress and [Abbrechen] →
 * treemap and the largest entries with bars; tap a folder → into it; back → up; ⋮ "In Dateien
 * öffnen"; "n Pfade nicht lesbar [Bericht]"; the analysis' notes (for a remote place the other
 * device's, e.g. areas it protects); areas Android locks are counted apart, sized by the
 * platform where possible, with the usage access card for the other apps' folders. With usage
 * access the internal storage's root lists "≈ Apps (laut Android)": tap → one row per app, tap an
 * app → its breakdown with [App-Info öffnen].
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
            preparing = vm.preparing,
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
                node.children.isEmpty() -> EmptyFolder(node, vm, onReport = { report = it })
                else -> NodeList(node, vm, onReport = { report = it })
            }
        }
    }
    vm.appDetail?.let { app -> AppDetailDialog(app, onDismiss = { vm.closeAppDetail() }) }
    val issues = vm.issues
    when {
        issues == null -> Unit
        report == Report.Issues -> TextReportDialog("Leseprobleme", issues.text, onDismiss = { report = null })
        report == Report.Protected -> TextReportDialog(
            "Geschützte Bereiche",
            issues.protectedText.ifBlank { PROTECTED_EXPLANATION },
            onDismiss = { report = null },
        )
        report == Report.Notes -> TextReportDialog("Hinweise", issues.notes.joinToString("\n\n"), onDismiss = { report = null })
    }
}

/**
 * The analysis' notes at the analysed root (shown even without read problems): the first ones as
 * they are, the rest behind "Alle Hinweise".
 */
@Composable
private fun NotesRows(vm: AnalysisViewModel, onReport: (Report) -> Unit) {
    val notes = vm.issues?.notes.orEmpty()
    if (notes.isEmpty() || vm.path.isNotEmpty()) return
    val tint = MaterialTheme.colorScheme.onSurfaceVariant
    notes.take(INLINE_NOTES).forEach { note -> NoticeRow(R.drawable.ic_info, note, tint = tint) }
    if (notes.size > INLINE_NOTES) {
        NoticeRow(
            R.drawable.ic_info,
            "${notes.size - INLINE_NOTES} weitere Hinweise",
            tint = tint,
            actionLabel = "Alle Hinweise",
            onAction = { onReport(Report.Notes) },
        )
    }
}

/** Shows the usage access card where the other apps' folders matter: the root and locked folders. */
private fun showsUsageCard(vm: AnalysisViewModel): Boolean =
    vm.appDataInTree && !vm.usageHintDismissed && (vm.path.isEmpty() || vm.insideProtected)

/** An empty folder; a locked one (or a locked analysed root) says why it is empty. */
@Composable
private fun EmptyFolder(node: AnalyzeNode, vm: AnalysisViewModel, onReport: (Report) -> Unit) {
    val lockedRoot = vm.path.isEmpty() && (vm.issues?.protectedCount ?: 0L) > 0L
    if (!vm.insideProtected && !lockedRoot) {
        Column(Modifier.fillMaxSize()) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) { NotesRows(vm, onReport) }
            EmptyState(R.drawable.ic_folder, "Dieser Ordner ist leer", modifier = Modifier.weight(1f), message = Format.size(node.size))
        }
        return
    }
    Column(Modifier.fillMaxSize()) {
        Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) { NotesRows(vm, onReport) }
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
    // node.size holds the estimates below it, with app folders that are also inside "≈ Apps" once.
    val total = remember(node) { if (node.size > 0) node.size else node.children.sumOf { it.size } }
    val estimated = remember(node) { (node.size - node.measured).coerceAtLeast(0) }
    val counted = remember(node) { (node.children.sumOf { it.size } - total).coerceAtLeast(0) }
    val hasApps = remember(node) { node.children.any { it.rowKind == AnalyzeKind.Apps } }
    val entries = remember(node) { node.children.count { !it.isEstimate } }
    LazyColumn(Modifier.fillMaxSize()) {
        item(key = "header") {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                val where = if (vm.path.isEmpty()) vm.location.orEmpty() else vm.path.joinToString(" › ")
                Text(where, style = MaterialTheme.typography.bodySmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text("${Format.size(total)} · $entries ${if (node.isAppList) "Apps" else "Einträge"}", style = MaterialTheme.typography.titleSmall)
                when {
                    node.isAppList -> Hint(APP_LIST_EXPLANATION)
                    estimated > 0 -> Hint("davon ≈ ${Format.size(estimated)} laut Android (nicht als Dateien erfasst)")
                }
                // Rows add up to more than the whole by what both "Android" and "≈ Apps" hold; said
                // once it moves the shown percentages (≥ 1 %).
                if (hasApps && counted > 0 && counted * 100 >= total) {
                    Hint("≈ ${Format.size(counted)} in App-Ordnern unter „Android“ sind auch in „≈ Apps“ enthalten (einmal gezählt)")
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
                NotesRows(vm, onReport)
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

/** What the app list shows and where the apps' own settings are. */
private const val APP_LIST_EXPLANATION =
    "Größen laut Android: App (APK/Code) plus Daten (inkl. Android/data) je App. Antippen zeigt die " +
        "Aufteilung und öffnet die App-Info, z. B. zum Cache leeren."

@Composable
private fun Hint(text: String) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}
