// material3 1.4.0: several components may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.sync

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.api.MirrorResult
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.api.SyncApi.failureText
import app.smartexplorer.android.api.SyncApi.resultAs
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.rememberSavedTaskId
import app.smartexplorer.android.ui.picker.LocationPickerDialog
import kotlinx.coroutines.launch

/**
 * "Spiegeln nach…" (spec F14): one-way copy of new and changed files from [source] into a chosen
 * target; nothing is deleted in the target and nothing is stored. Without [source] the folder
 * is chosen first. Steps: pick → confirm → progress with [Abbrechen]/[Im Hintergrund] → result
 * "n kopiert, n übersprungen, n Fehler" with [Als Sync-Job speichern] (opens the editor prefilled).
 */
@Composable
fun MirrorDialog(source: String?, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    var from by rememberSaveable { mutableStateOf(source) }
    var to by rememberSaveable { mutableStateOf<String?>(null) }
    // Survives recreation; after process death the mirror is gone and the dialog asks again.
    var taskId by rememberSavedTaskId()
    // Not saved: the start call does not survive a recreation, so a restored `true` would block
    // [Spiegeln] for good.
    var starting by remember { mutableStateOf(false) }
    val volumes = rememberVolumes()
    val tasks by Core.tasks.collectAsStateWithLifecycle()
    val chosenFrom = from
    val chosenTo = to
    val runningId = taskId

    when {
        chosenFrom == null -> LocationPickerDialog(
            title = "Ordner zum Spiegeln",
            initialLocation = null,
            confirmLabel = "Auswählen",
            onPick = { from = it },
            onDismiss = onDismiss,
        )
        chosenTo == null -> LocationPickerDialog(
            title = "Spiegeln nach…",
            initialLocation = null,
            confirmLabel = "Hierhin",
            onPick = { to = it },
            onDismiss = onDismiss,
        )
        runningId == null -> AlertDialog(
            onDismissRequest = onDismiss,
            title = { Text("Spiegeln") },
            text = {
                Text(
                    "Neues und Geändertes aus ${SyncLocations.label(chosenFrom, volumes)} wird nach " +
                        "${SyncLocations.label(chosenTo, volumes)} kopiert; im Ziel wird nichts gelöscht.",
                )
            },
            confirmButton = {
                TextButton(
                    enabled = !starting,
                    onClick = {
                        starting = true
                        scope.launch {
                            try {
                                taskId = SyncApi.mirror(chosenFrom, chosenTo)
                            } catch (e: CoreException) {
                                Snackbars.show("Spiegeln nicht gestartet: ${e.displayText()}")
                                onDismiss()
                            } finally {
                                starting = false
                            }
                        }
                    },
                ) { Text("Spiegeln") }
            },
            dismissButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } },
        )
        else -> MirrorProgress(
            task = tasks.firstOrNull { it.id == runningId },
            onCancel = {
                scope.launch {
                    try {
                        SyncApi.cancelTask(runningId)
                    } catch (e: CoreException) {
                        Snackbars.show("Nicht abgebrochen: ${e.displayText()}")
                    }
                }
            },
            onSaveAsJob = {
                onDismiss()
                SyncLaunch.newJob(chosenFrom, chosenTo)
            },
            onDismiss = onDismiss,
        )
    }
}

@Composable
private fun MirrorProgress(task: TaskInfo?, onCancel: () -> Unit, onSaveAsJob: () -> Unit, onDismiss: () -> Unit) {
    val finished = task != null && !task.isActive
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (finished) "Spiegeln beendet" else "Spiegeln läuft") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (task == null || task.isActive) {
                    val fraction = task?.let { Format.fraction(it.doneBytes, it.totalBytes) }
                    if (fraction != null) {
                        LinearProgressIndicator(progress = { fraction }, modifier = Modifier.fillMaxWidth())
                    } else {
                        LinearProgressIndicator(Modifier.fillMaxWidth())
                    }
                    Text(progressText(task), style = MaterialTheme.typography.bodyMedium)
                } else {
                    Text(resultText(task), style = MaterialTheme.typography.bodyMedium)
                    task.resultAs<MirrorResult>()?.omitted?.takeIf { it.isNotBlank() }?.let {
                        Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
        },
        confirmButton = {
            if (finished && task?.state == "done") {
                TextButton(onClick = onSaveAsJob) { Text("Als Sync-Job speichern") }
            } else if (!finished) {
                TextButton(onClick = onCancel) { Text("Abbrechen") }
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(if (finished) "Schließen" else "Im Hintergrund") }
        },
    )
}

private fun progressText(task: TaskInfo?): String {
    if (task == null) return "Startet…"
    val parts = mutableListOf<String>()
    if (task.totalItems > 0) parts += "${task.doneItems} von ${task.totalItems} Dateien"
    if (task.totalBytes > 0) parts += "${Format.size(task.doneBytes)} von ${Format.size(task.totalBytes)}"
    if (task.rateBps > 0) parts += Format.rate(task.rateBps)
    return parts.joinToString(" · ").ifEmpty { task.message?.ifBlank { null } ?: "Läuft…" }
}

private fun resultText(task: TaskInfo): String {
    if (task.state != "done") return task.failureText()
    val result = task.resultAs<MirrorResult>() ?: return task.message?.ifBlank { null } ?: "Fertig"
    return "${result.copied} kopiert, ${result.skipped} übersprungen, ${result.errors} Fehler"
}
