// material3 1.4.0: TopAppBar may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.share

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.viewmodel.compose.viewModel
import app.smartexplorer.android.R
import app.smartexplorer.android.api.ExecResult
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.more.SectionHeader
import app.smartexplorer.android.ui.more.TextActions
import app.smartexplorer.android.ui.more.ToggleSetting

/** Timeouts offered for a remote command, in seconds. */
private val EXEC_TIMEOUTS = listOf(30, 60, 300, 900)

/** Finished run: the result on success, otherwise the reason. */
private data class ExecOutcome(val result: ExecResult?, val failure: String?)

/**
 * "Befehl ausführen" on a shared device (spec F18): command, shell or direct, time limit →
 * `share.exec` task → stdout/stderr and exit code, [Abbrechen] while it runs. The dialog leaving
 * the screen in any way (close, back, tab switch, recreation) cancels a running or starting command.
 */
@Composable
internal fun ExecDialog(
    targetName: String,
    location: String,
    onDismiss: () -> Unit,
    vm: ShareViewModel = viewModel { ShareViewModel() },
) {
    val context = LocalContext.current
    var command by rememberSaveable { mutableStateOf("") }
    var shell by rememberSaveable { mutableStateOf(true) }
    var timeout by rememberSaveable { mutableStateOf(60) }
    var taskId by remember { mutableStateOf<String?>(null) }
    var outcome by remember { mutableStateOf<ExecOutcome?>(null) }
    // `share.exec` has not returned the task id yet; a cancel meanwhile waits for the id.
    var starting by remember { mutableStateOf(false) }
    var cancelWanted by remember { mutableStateOf(false) }
    val running = starting || (taskId != null && outcome == null)

    LaunchedEffect(taskId) {
        val id = taskId ?: return@LaunchedEffect
        outcome = try {
            outcomeOf(FilesApi.awaitTask(id))
        } catch (e: CoreException) {
            ExecOutcome(null, e.message ?: e.kind)
        }
    }

    fun cancelRunning() {
        if (starting) {
            cancelWanted = true
            return
        }
        val id = taskId ?: return
        if (outcome != null) return
        vm.act("Nicht abgebrochen") { FilesApi.cancelTask(id) }
    }

    fun start() {
        // Checked on the state itself: a second tap can arrive before the button turns into [Abbrechen].
        if (starting || (taskId != null && outcome == null)) return
        starting = true
        cancelWanted = false
        taskId = null
        outcome = null
        vm.startExec(location, command.trim(), shell, timeout, cancelNow = { cancelWanted }) { id, failure ->
            starting = false
            if (id != null) {
                taskId = id
            } else {
                outcome = ExecOutcome(null, failure)
            }
        }
    }

    // The only cancel path on leaving, so a running command never outlives the dialog unseen.
    DisposableEffect(Unit) {
        onDispose { cancelRunning() }
    }

    val close = onDismiss
    Dialog(onDismissRequest = close, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surface) {
            Column(Modifier.fillMaxSize().navigationBarsPadding().imePadding()) {
                TopAppBar(
                    title = { Text("Befehl auf $targetName", maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    navigationIcon = { IconButton(onClick = close) { SeIcon(R.drawable.ic_close, contentDescription = "Schließen") } },
                )
                Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                    Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedTextField(
                            value = command,
                            onValueChange = { command = it },
                            label = { Text("Befehl") },
                            minLines = 2,
                            enabled = !running,
                            textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
                            modifier = Modifier.fillMaxWidth(),
                        )
                        Text("Zeitlimit", style = MaterialTheme.typography.labelLarge)
                        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            EXEC_TIMEOUTS.forEach { seconds ->
                                FilterChip(
                                    selected = timeout == seconds,
                                    onClick = { timeout = seconds },
                                    enabled = !running,
                                    label = { Text(if (seconds < 60) "$seconds s" else "${seconds / 60} min") },
                                )
                            }
                        }
                    }
                    ToggleSetting("Über die Shell des Geräts", shell, { shell = it }, enabled = !running)
                    Row(Modifier.padding(16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        if (running) {
                            OutlinedButton(onClick = { cancelRunning() }) { Text("Abbrechen") }
                        } else {
                            Button(onClick = { start() }, enabled = command.isNotBlank()) { Text("Ausführen") }
                        }
                    }
                    if (running) LinearProgressIndicator(modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp))
                    outcome?.let { ExecOutput(it, onCopy = { text -> TextActions.copy(context, "Ausgabe", text) }) }
                }
            }
        }
    }
}

private fun outcomeOf(task: TaskInfo): ExecOutcome = when (task.state) {
    "done" -> try {
        ExecOutcome(FilesApi.resultOf(task, ExecResult.serializer()) ?: ExecResult(), null)
    } catch (e: CoreException) {
        ExecOutcome(null, e.message ?: e.kind)
    }
    "canceled" -> ExecOutcome(null, "Abgebrochen")
    else -> ExecOutcome(null, task.message?.takeIf { it.isNotBlank() } ?: "Befehl nicht ausgeführt")
}

@Composable
private fun ExecOutput(outcome: ExecOutcome, onCopy: (String) -> Unit) {
    val result = outcome.result
    if (result == null) {
        Text(
            outcome.failure.orEmpty(),
            color = MaterialTheme.colorScheme.error,
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(horizontal = 16.dp),
        )
        return
    }
    val summary = buildList {
        add(result.exitCode?.let { "Exitcode $it" } ?: "Kein Exitcode")
        if (result.timedOut) add("Zeitlimit erreicht")
        if (result.truncated) add("Ausgabe gekürzt")
    }.joinToString(" · ")
    Text(summary, style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(horizontal = 16.dp))
    OutputBlock("Ausgabe (stdout)", result.stdout)
    if (result.stderr.isNotEmpty()) OutputBlock("Fehlerausgabe (stderr)", result.stderr)
    OutlinedButton(
        onClick = { onCopy(buildString { append(result.stdout); if (result.stderr.isNotEmpty()) append("\n").append(result.stderr) }) },
        modifier = Modifier.padding(16.dp),
    ) { Text("Kopieren") }
}

@Composable
private fun OutputBlock(title: String, text: String) {
    SectionHeader(title)
    SelectionContainer {
        Text(
            text.ifEmpty { "(leer)" },
            fontFamily = FontFamily.Monospace,
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(horizontal = 16.dp).horizontalScroll(rememberScrollState()),
        )
    }
}
