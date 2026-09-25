// material3 1.4.0: several components may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.sync

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import app.smartexplorer.android.R
import app.smartexplorer.android.api.SyncJob
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.service.BackgroundController
import app.smartexplorer.android.service.BackgroundText
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.ui.common.ConfirmDialog
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.settings.BackgroundSettingsSection

private const val STATUS_INTERVAL_MS = 10_000L

/**
 * Tab "Sync" (spec F14–F17): background status, job cards, editor, conflict list, line merge and
 * the background settings as sub-pages. Handles `NavRequest.OpenJobConflicts` from notifications
 * and job prefills from the mirror dialog ([SyncLaunch]).
 */
@Composable
fun SyncScreen() {
    val vm = viewModel { SyncViewModel() }
    val pending by AppNav.pending.collectAsStateWithLifecycle()
    val prefill by SyncLaunch.pending.collectAsStateWithLifecycle()

    LaunchedEffect(pending) {
        val request = pending
        if (request is NavRequest.OpenJobConflicts) {
            vm.openConflictsById(request.jobId)
            AppNav.consume(request)
        }
    }
    LaunchedEffect(prefill) {
        val job = prefill ?: return@LaunchedEffect
        vm.openEditor(prefill = job)
        SyncLaunch.consume(job)
    }
    BackHandler(enabled = vm.page != SyncPage.Jobs) { vm.back() }

    when (val page = vm.page) {
        SyncPage.Jobs -> JobsPage(vm)
        is SyncPage.Editor -> JobEditorScreen(page.draft, onSave = { vm.save(page.draft) }, onClose = vm::back)
        is SyncPage.Conflicts -> ConflictsScreen(
            session = page.session,
            onBack = vm::back,
            onAbandon = vm::abandonConflicts,
            onMerge = { vm.openMerge(page.session, it) },
        )
        is SyncPage.Merge -> MergeScreen(page.merge, onBack = vm::back)
        SyncPage.Background -> BackgroundPage(onBack = vm::back)
    }
    vm.details?.let { text -> DetailsDialog(text, onDismiss = { vm.details = null }) }
}

@Composable
private fun JobsPage(vm: SyncViewModel) {
    val context = LocalContext.current
    val mode by AppPrefs.bgMode.collectAsStateWithLifecycle()
    val nextRun by remember(context) { BackgroundController.nextRun(context) }.collectAsStateWithLifecycle(initialValue = null)
    val tasks by Core.tasks.collectAsStateWithLifecycle()
    val volumes = rememberVolumes()
    var mirror by rememberSaveable { mutableStateOf(false) }
    var deleting by remember { mutableStateOf<SyncJob?>(null) }
    LaunchedEffect(Unit) { vm.reloadJobs() }
    RepeatWhileStarted(STATUS_INTERVAL_MS) { vm.loadStatus() }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Sync") },
                actions = {
                    IconButton(onClick = vm::refresh) { SeIcon(R.drawable.ic_refresh, contentDescription = "Aktualisieren") }
                    PageMenu(onMirror = { mirror = true }, onBackground = vm::showBackground)
                },
            )
        },
        floatingActionButton = {
            if (vm.jobs.isNotEmpty()) {
                FloatingActionButton(onClick = { vm.openEditor() }) { SeIcon(R.drawable.ic_add, contentDescription = "Job anlegen") }
            }
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            LoadingBar(vm.loading)
            LazyColumn(
                contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 96.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                item(key = "header") {
                    BackgroundHeaderCard(BackgroundText.summary(mode, vm.status, nextRun), onOpen = vm::showBackground)
                }
                vm.loadError?.let { error ->
                    item(key = "error") {
                        ErrorCard(error, title = "Jobs nicht geladen", actionLabel = "Erneut laden", onAction = vm::refresh)
                    }
                }
                if (vm.loaded && vm.jobs.isEmpty()) {
                    item(key = "empty") { NoJobs(onCreate = { vm.openEditor() }, onMirror = { mirror = true }) }
                }
                items(vm.jobs, key = { it.id }) { job ->
                    val taskId = job.runningTask ?: vm.startedRuns[job.id]
                    val task = taskId?.let { id -> tasks.firstOrNull { it.id == id } }
                    JobCard(
                        job = job,
                        volumes = volumes,
                        running = taskId != null && (task == null || task.isActive),
                        runTask = task,
                        daemonRuns = vm.status?.activeJob.let { it != null && it == job.name.ifBlank { job.id } },
                        actions = JobActions(
                            onRun = { vm.runNow(job) },
                            onEdit = { vm.openEditor(job) },
                            onToggle = { vm.setEnabled(job, !job.enabled) },
                            onConflicts = { vm.openConflicts(job) },
                            onDelete = { deleting = job },
                        ),
                    )
                }
            }
        }
    }

    if (mirror) MirrorDialog(source = null, onDismiss = { mirror = false })
    deleting?.let { job ->
        ConfirmDialog(
            title = "Job löschen?",
            message = "„${job.name}“ wird gelöscht. Die Dateien auf beiden Seiten bleiben unverändert.",
            confirmLabel = "Löschen",
            onConfirm = {
                deleting = null
                vm.delete(job)
            },
            onDismiss = { deleting = null },
            destructive = true,
        )
    }
}

@Composable
private fun PageMenu(onMirror: () -> Unit, onBackground: () -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { SeIcon(R.drawable.ic_more_vert, contentDescription = "Weitere Aktionen") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(
                text = { Text("Ordner spiegeln") },
                onClick = {
                    open = false
                    onMirror()
                },
            )
            DropdownMenuItem(
                text = { Text("Hintergrund") },
                onClick = {
                    open = false
                    onBackground()
                },
            )
        }
    }
}

@Composable
private fun BackgroundPage(onBack: () -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Hintergrund") },
                navigationIcon = {
                    IconButton(onClick = onBack) { SeIcon(R.drawable.ic_arrow_back, contentDescription = "Zurück") }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(start = 16.dp, end = 16.dp, bottom = 24.dp),
        ) {
            BackgroundSettingsSection()
        }
    }
}

/** [Details] of a failed run or resolution; the text can be selected and copied. */
@Composable
private fun DetailsDialog(text: String, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Details") },
        text = {
            Box(Modifier.heightIn(max = 360.dp)) {
                SelectionContainer { Text(text, modifier = Modifier.verticalScroll(rememberScrollState())) }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Schließen") } },
    )
}
