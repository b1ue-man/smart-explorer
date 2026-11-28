package app.smartexplorer.android.ui.files

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/**
 * The confirmation of "In Dateien ersetzen": asking "Ersetzen durch" (from the result), counting,
 * then folder, filter, places and files. "Abbrechen" is the emphasized button and closing the
 * dialog cancels, as Notepad++ preselects "Abbrechen"; nothing is written before "Ersetzen".
 */
@Composable
internal fun ReplaceConfirmDialog(confirm: ReplaceConfirm, flow: ContentReplaceFlow) {
    val stage = confirm.stage
    var text by remember(confirm.job) { mutableStateOf((stage as? ReplaceStage.Asking)?.text ?: confirm.job.replacement) }
    AlertDialog(
        onDismissRequest = { flow.dismissConfirm() },
        title = { Text(if (stage is ReplaceStage.Asking) "Ausgewählte Treffer ersetzen" else "Wirklich ersetzen?") },
        text = {
            Column(Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                when (stage) {
                    is ReplaceStage.Asking -> {
                        Text("${confirm.job.chosenPlaces ?: 0} ausgewählte Stellen in ${confirm.job.location}")
                        OutlinedTextField(
                            value = text,
                            onValueChange = { text = it },
                            label = { Text("Ersetzen durch") },
                            singleLine = true,
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                    is ReplaceStage.Counting -> Row(verticalAlignment = Alignment.CenterVertically) {
                        CircularProgressIndicator(Modifier.padding(end = 12.dp))
                        Text("Vorschau läuft: ${stage.replacements} Stellen in ${stage.files} Dateien …")
                    }
                    is ReplaceStage.Failed -> Text(stage.message, color = MaterialTheme.colorScheme.error)
                    is ReplaceStage.Ready -> {
                        if (stage.replacements == 0L) {
                            Text("Keine Stelle zu ersetzen.")
                        } else {
                            confirmLines(confirm.job, stage).forEachIndexed { index, line ->
                                Text(line, style = if (index == 1) MaterialTheme.typography.titleSmall else MaterialTheme.typography.bodyMedium)
                            }
                        }
                        if (stage.skipped.isNotEmpty()) {
                            Text("Übersprungen (${stage.skipped.size})", style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(top = 8.dp))
                            stage.skipped.forEach {
                                Text("${it.rel}: ${it.reason}", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        }
                    }
                }
            }
        },
        confirmButton = {
            when {
                stage is ReplaceStage.Asking -> TextButton(onClick = { flow.previewWith(text) }) { Text("Weiter") }
                stage is ReplaceStage.Ready && stage.replacements > 0 -> TextButton(onClick = { flow.apply() }) { Text("Ersetzen") }
                else -> {}
            }
        },
        dismissButton = {
            val label = if (stage is ReplaceStage.Failed || (stage is ReplaceStage.Ready && stage.replacements == 0L)) "Schließen" else "Abbrechen"
            Button(onClick = { flow.dismissConfirm() }) { Text(label) }
        },
    )
}

/** The outcome of the last run: totals, skipped files with their reasons, [Abbrechen] and [Rückgängig]. */
@Composable
internal fun ReplaceOutcomeCard(flow: ContentReplaceFlow) {
    val outcome = flow.outcome ?: return
    // A state read here: the card follows every applied poll answer.
    val header = remember(outcome, flow.version) { outcome.header() }
    val skipped = remember(outcome, flow.version) { outcome.skipped.toList() }
    val undo = remember(outcome, flow.version) { outcome.undo }
    Surface(
        color = MaterialTheme.colorScheme.secondaryContainer,
        shape = MaterialTheme.shapes.medium,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
    ) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(header, style = MaterialTheme.typography.titleSmall)
            val left = (undo as? UndoStage.Partly)?.left.orEmpty()
            (skipped + left).forEach {
                Text("${it.rel}: ${it.reason}", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (flow.running) OutlinedButton(onClick = { flow.cancel() }) { Text("Abbrechen") }
                if (undo is UndoStage.Available && !flow.running) {
                    OutlinedButton(onClick = { flow.undo() }) { Text("Rückgängig") }
                }
            }
        }
    }
}

/** Shown while result rows are selected (long press): replace or dismiss them. */
@Composable
internal fun SelectionBar(state: ContentSearchState) {
    if (state.selected.isEmpty()) return
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text("${state.selected.size} ausgewählt", modifier = Modifier.weight(1f))
        TextButton(onClick = { state.replaceSelected() }) { Text("Ersetzen…") }
        TextButton(onClick = { state.dismissSelected() }) { Text("Entfernen") }
        TextButton(onClick = { state.clearSelection() }) { Text("Aufheben") }
    }
}
