// material3 1.4.0: several components may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.sync

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import app.smartexplorer.android.api.BgStatus
import app.smartexplorer.android.api.CatchUpResult
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.api.SyncApi.resultAs
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.service.BackgroundText
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

// Building blocks of the background settings (spec F17), also used by the Sync page header.

/** Runs [action] now and every [intervalMs] while the UI is at least STARTED. */
@Composable
internal fun RepeatWhileStarted(intervalMs: Long, action: suspend () -> Unit) {
    val owner = LocalLifecycleOwner.current
    val current by rememberUpdatedState(action)
    LaunchedEffect(owner, intervalMs) {
        owner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                current()
                delay(intervalMs)
            }
        }
    }
}

/** Status lines: mode summary, running job, last catch-up and daemon state. */
@Composable
internal fun BackgroundStatusBlock(mode: String, status: BgStatus?, nextRunMs: Long?, error: String?) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(BackgroundText.summary(mode, status, nextRunMs), style = MaterialTheme.typography.bodyLarge)
        if (status != null) {
            val last = status.lastCatchUpMs
            HintText(if (last != null) "Letzter Hintergrundlauf: ${BackgroundText.moment(last)}" else "Noch kein Hintergrundlauf")
            if (!status.daemonRunning) HintText("Der Hintergrund-Worker startet noch.")
        }
        if (error != null) ErrorText("Status nicht verfügbar: $error")
    }
}

/**
 * [Jetzt nachholen] plus the outcome of the newest catch-up run in this process: its message
 * and the jobs the supervisor did not admit, each with its reason (B1b semantics).
 */
@Composable
internal fun CatchUpBlock(onChanged: () -> Unit) {
    val scope = rememberCoroutineScope()
    val tasks by Core.tasks.collectAsStateWithLifecycle()
    val latest = tasks.lastOrNull { it.kind == SyncApi.KIND_CATCH_UP }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedButton(
                onClick = {
                    scope.launch {
                        try {
                            SyncApi.catchUp()
                        } catch (e: CoreException) {
                            Snackbars.show("Nachholen nicht gestartet: ${e.displayText()}")
                        }
                        onChanged()
                    }
                },
                enabled = latest?.isActive != true,
            ) { Text("Jetzt nachholen") }
            HintText("Fällige Intervall- und Zeitplan-Jobs, Echtzeit-Jobs einmal.", Modifier.weight(1f))
        }
        if (latest != null) CatchUpOutcome(latest)
    }
}

@Composable
private fun CatchUpOutcome(task: TaskInfo) {
    if (task.isActive) {
        Text(task.message?.ifBlank { null } ?: "Nachholen läuft…", style = MaterialTheme.typography.bodyMedium)
        LinearProgressIndicator(Modifier.fillMaxWidth())
        return
    }
    val result = task.resultAs<CatchUpResult>()
    val headline = task.message?.ifBlank { null } ?: when (task.state) {
        "done" -> "Nachholen beendet"
        "canceled" -> "Nachholen abgebrochen"
        else -> "Nachholen fehlgeschlagen"
    }
    Text(headline, style = MaterialTheme.typography.bodyMedium)
    result?.skipped?.forEach { skip ->
        HintText("Übersprungen: ${skip.jobName.ifBlank { skip.jobId }} – ${skip.reason}")
    }
}

/** Worker log (`bg.log`), selectable for copying. */
@Composable
internal fun WorkerLogDialog(onDismiss: () -> Unit) {
    var text by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var reloads by remember { mutableIntStateOf(0) }
    LaunchedEffect(reloads) {
        try {
            text = SyncApi.log(LOG_BYTES)
            error = null
        } catch (e: CoreException) {
            error = e.displayText()
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Worker-Protokoll") },
        text = {
            Box(Modifier.heightIn(max = 420.dp)) {
                val current = text
                when {
                    error != null -> ErrorText("Protokoll nicht lesbar: $error")
                    current == null -> LinearProgressIndicator(Modifier.fillMaxWidth().padding(vertical = 16.dp))
                    current.isBlank() -> Text("Das Protokoll ist leer.")
                    else -> SelectionContainer {
                        Text(
                            current,
                            style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                            modifier = Modifier.verticalScroll(rememberScrollState()),
                        )
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Schließen") } },
        dismissButton = { TextButton(onClick = { reloads++ }) { Text("Aktualisieren") } },
    )
}

private const val LOG_BYTES = 64 * 1024
