@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
package app.smartexplorer.android.ui.sync

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.api.SyncJob
import app.smartexplorer.android.ui.common.SeIcon
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Lines kept on screen (memory bound); the log file on the device keeps more. */
private const val MAX_LINES = 20_000
private const val REFRESH_MS = 1_000L

/**
 * Live log of one sync job: every listed folder, comparison and action of every runner
 * (background worker, "Jetzt", desktop and terminal on this device), refreshed every second.
 */
@Composable
internal fun SyncLogScreen(job: SyncJob, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    val lines = remember(job.id) { mutableStateListOf<String>() }
    var next by remember(job.id) { mutableStateOf<Long?>(null) }
    var size by remember(job.id) { mutableLongStateOf(0L) }
    var verbose by remember(job.id) { mutableStateOf(false) }
    var follow by remember { mutableStateOf(true) }
    var filter by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(job.id) {
        while (true) {
            try {
                val chunk = SyncApi.log(job.id, next)
                if (chunk.restarted) lines.clear()
                if (chunk.text.isNotEmpty()) lines.addAll(chunk.text.trimEnd('\n').split('\n'))
                if (lines.size > MAX_LINES) {
                    val keep = lines.takeLast(MAX_LINES)
                    lines.clear()
                    lines.addAll(keep)
                }
                next = chunk.next
                size = chunk.size
                verbose = chunk.verbose
                error = null
            } catch (e: Exception) {
                if (e is kotlinx.coroutines.CancellationException) throw e
                error = e.displayText()
            }
            delay(REFRESH_MS)
        }
    }
    val shown = if (filter.isBlank()) lines.toList() else lines.filter { it.contains(filter, ignoreCase = true) }
    val listState = rememberLazyListState()
    LaunchedEffect(shown.size, follow) {
        if (follow && shown.isNotEmpty()) listState.scrollToItem(shown.size - 1)
    }
    Scaffold(topBar = {
        TopAppBar(title = { Text("Protokoll: ${job.name.ifBlank { job.id }}") }, navigationIcon = {
            IconButton(onClick = onBack) { SeIcon(R.drawable.ic_arrow_back, "Zurück") }
        })
    }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).padding(horizontal = 12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Switch(checked = follow, onCheckedChange = { follow = it })
                Text("Mitlaufen")
            }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Switch(checked = verbose, onCheckedChange = { wanted ->
                    scope.launch {
                        try {
                            SyncApi.setLogVerbose(job.id, wanted)
                            verbose = wanted
                        } catch (e: Exception) {
                            if (e is kotlinx.coroutines.CancellationException) throw e
                            error = e.displayText()
                        }
                    }
                })
                Text("Unveränderte Einträge einzeln protokollieren (ab dem nächsten Lauf)")
            }
            OutlinedTextField(
                value = filter,
                onValueChange = { filter = it },
                label = { Text("Filter") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            error?.let { ErrorText("Protokoll nicht lesbar: $it") }
            HintText(
                if (lines.isEmpty()) "Noch keine Einträge. Der nächste Lauf dieses Syncs schreibt hier jeden Schritt."
                else "${shown.size} von ${lines.size} Zeilen · Datei ${"%.1f".format(size / 1048576.0)} MB · jede Sekunde aktualisiert",
            )
            LazyColumn(state = listState, contentPadding = PaddingValues(bottom = 12.dp), modifier = Modifier.fillMaxSize()) {
                itemsIndexed(shown) { _, line ->
                    Text(
                        line,
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                        color = lineColor(line),
                    )
                }
            }
        }
    }
}

@Composable
private fun lineColor(line: String): Color {
    val tag = line.drop(24).trimStart().substringBefore(' ')
    return when (tag) {
        "Fehler", "Stopp" -> MaterialTheme.colorScheme.error
        "Unterbrochen", "Verschoben", "Ausgelassen", "Gestoppt", "Wiederholung" -> MaterialTheme.colorScheme.tertiary
        "Aktion", "Ergebnis", "Ende", "Start", "Auslöser", "Vorschau" -> MaterialTheme.colorScheme.primary
        else -> MaterialTheme.colorScheme.onSurface
    }
}
