package app.smartexplorer.android.ui.analytics

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedCard
import androidx.compose.material3.Text
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
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.more.rememberTask
import app.smartexplorer.android.ui.picker.LocationPickerDialog

/**
 * First page of analysis and duplicate search: place (target picker), tool [options], start
 * button, error of the last attempt.
 */
@Composable
internal fun ScanSetupPage(
    title: String,
    location: String?,
    onLocation: (String) -> Unit,
    error: String?,
    startLabel: String,
    onStart: () -> Unit,
    onClose: () -> Unit,
    hint: String,
    options: @Composable () -> Unit = {},
) {
    var picking by remember { mutableStateOf(false) }
    SubPageScaffold(title = title, onBack = onClose) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("Ort", style = MaterialTheme.typography.labelLarge)
            OutlinedCard(onClick = { picking = true }, modifier = Modifier.fillMaxWidth()) {
                Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    SeIcon(R.drawable.ic_folder, contentDescription = null)
                    Text(
                        location ?: "Ort wählen",
                        style = MaterialTheme.typography.bodyLarge,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    SeIcon(R.drawable.ic_chevron_right, contentDescription = null)
                }
            }
            options()
            Button(onClick = onStart, enabled = location != null) { Text(startLabel) }
            error?.let { ErrorCard(it, title = "Nicht abgeschlossen") }
            Text(hint, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
    if (picking) {
        LocationPickerDialog(
            title = "Ort wählen",
            initialLocation = location,
            confirmLabel = "Wählen",
            onPick = {
                picking = false
                onLocation(it)
            },
            onDismiss = { picking = false },
        )
    }
}

/** Running scan: place, files and bytes so far, the core's progress text and [Abbrechen]. */
@Composable
internal fun ScanProgressPage(title: String, location: String?, taskId: String?, onCancel: () -> Unit, onClose: () -> Unit) {
    val task = rememberTask(taskId)
    SubPageScaffold(title = title, onBack = onClose) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            location?.let { Text(it, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis) }
            LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
            Text(progressLine(task), style = MaterialTheme.typography.titleMedium)
            task?.message?.takeIf { it.isNotBlank() }?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
            OutlinedButton(onClick = onCancel, enabled = taskId != null) { Text("Abbrechen") }
            Text(
                "Der Scan läuft weiter, wenn diese Seite verlassen wird.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

private fun progressLine(task: TaskInfo?): String = when {
    task == null || task.state == "queued" -> "Wird gestartet …"
    else -> "${task.doneItems} Dateien · ${Format.size(task.doneBytes)}"
}
