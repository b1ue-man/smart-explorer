package app.smartexplorer.android.ui.transfers

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.R
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.system.Opener
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/**
 * Sheet "Übertragungen" (spec F8): running transfers with progress and [✕], [Alle abbrechen],
 * section "Fertig" with outcome, [Details] and [Leeren]. Swipe down closes it.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TransfersSheet(onDismiss: () -> Unit) {
    val tasks by Core.tasks.collectAsStateWithLifecycle()
    val shown = tasks.filter { TransferTasks.isTransfer(it) }
    val active = shown.filter { it.isActive }
    val finished = shown.filterNot { it.isActive }.sortedByDescending { it.finishedMs ?: it.startedMs }
    val scope = rememberCoroutineScope()
    var details by remember { mutableStateOf<TaskInfo?>(null) }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(start = 24.dp, end = 16.dp, bottom = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Übertragungen", style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
            if (active.isNotEmpty()) {
                TextButton(onClick = { scope.cancelAll(active) }) { Text("Alle abbrechen") }
            }
        }
        if (shown.isEmpty()) {
            Text(
                "Keine Übertragungen. Kopieren, Verschieben, Empfangen und Entpacken erscheinen hier.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 24.dp),
            )
        }
        LazyColumn(Modifier.fillMaxWidth()) {
            items(active, key = { it.id }) { task ->
                ActiveTaskRow(task, onCancel = { scope.cancelTask(task.id) })
            }
            if (finished.isNotEmpty()) {
                item(key = "finished-header") {
                    if (active.isNotEmpty()) HorizontalDivider()
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(start = 24.dp, end = 16.dp, top = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text("Fertig", style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
                        TextButton(onClick = { scope.clearFinished() }) { Text("Leeren") }
                    }
                }
                items(finished, key = { it.id }) { task ->
                    FinishedTaskRow(task, onDetails = { details = task })
                }
            }
        }
    }
    details?.let { TaskDetailsDialog(it, onDismiss = { details = null }) }
}

@Composable
private fun ActiveTaskRow(task: TaskInfo, onCancel: () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(start = 24.dp, end = 8.dp, top = 8.dp, bottom = 8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                task.title,
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = onCancel) { SeIcon(R.drawable.ic_close, contentDescription = "Abbrechen") }
        }
        val fraction = Format.fraction(task.doneBytes, task.totalBytes) ?: Format.fraction(task.doneItems, task.totalItems)
        val barModifier = Modifier.fillMaxWidth().padding(end = 16.dp)
        if (fraction != null) {
            LinearProgressIndicator(progress = { fraction }, modifier = barModifier)
        } else {
            LinearProgressIndicator(modifier = barModifier)
        }
        Text(
            TransferTasks.progressLine(task),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(top = 4.dp),
        )
    }
}

@Composable
private fun FinishedTaskRow(task: TaskInfo, onDetails: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(start = 24.dp, end = 8.dp, top = 4.dp, bottom = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val failed = task.state == "failed" || task.errors.isNotEmpty()
        SeIcon(
            if (failed) R.drawable.ic_error else R.drawable.ic_check,
            contentDescription = null,
            tint = if (failed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary,
        )
        Column(Modifier.weight(1f)) {
            Text(task.title, style = MaterialTheme.typography.bodyLarge, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(
                TransferTasks.outcomeLine(task),
                style = MaterialTheme.typography.bodySmall,
                color = if (failed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (task.errors.isNotEmpty() || task.message != null) {
            TextButton(onClick = onDetails) { Text("Details") }
        }
    }
}

/** Message and per-file errors of a task, copyable. */
@Composable
internal fun TaskDetailsDialog(task: TaskInfo, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val text = buildString {
        task.message?.let { appendLine(it) }
        if (task.errors.isNotEmpty() && task.message != null) appendLine()
        task.errors.forEach { appendLine("${it.path}: ${it.message}") }
    }.trimEnd()
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(task.title) },
        text = {
            Column(Modifier.heightIn(max = 400.dp).verticalScroll(rememberScrollState())) {
                Text(text, style = MaterialTheme.typography.bodyMedium)
            }
        },
        confirmButton = {
            TextButton(onClick = {
                if (Opener.copyText(context, task.title, text)) Snackbars.show("Kopiert")
                onDismiss()
            }) { Text("Kopieren") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Schließen") } },
    )
}

private fun CoroutineScope.cancelTask(taskId: String) {
    launch {
        try {
            FilesApi.cancelTask(taskId)
        } catch (e: CoreException) {
            Snackbars.show("Abbrechen fehlgeschlagen: ${e.message}")
        }
    }
}

private fun CoroutineScope.cancelAll(tasks: List<TaskInfo>) {
    val ids = tasks.map { it.id }
    launch {
        var failures = 0
        for (id in ids) {
            try {
                FilesApi.cancelTask(id)
            } catch (e: CoreException) {
                // Already finished tasks answer not_found; only real failures count.
                if (e.kind != "not_found") failures++
            }
        }
        if (failures > 0) Snackbars.show("$failures Vorgänge ließen sich nicht abbrechen")
    }
}

private fun CoroutineScope.clearFinished() {
    launch {
        try {
            FilesApi.clearFinishedTasks()
        } catch (e: CoreException) {
            Snackbars.show("Leeren fehlgeschlagen: ${e.message}")
        }
    }
}
