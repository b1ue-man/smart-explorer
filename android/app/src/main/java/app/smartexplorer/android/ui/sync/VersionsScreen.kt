@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
package app.smartexplorer.android.ui.sync

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.*
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.api.SyncApi.failureText
import app.smartexplorer.android.api.SyncApi.resultAs
import app.smartexplorer.android.service.BackgroundText
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.LoadingBar
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
internal fun VersionsScreen(job: SyncJob, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    var rows by remember(job.id) { mutableStateOf<List<SyncVersion>>(emptyList()) }
    var reload by remember { mutableIntStateOf(0) }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var selected by remember { mutableStateOf<SyncVersion?>(null) }
    var legacySide by remember { mutableStateOf("a") }
    LaunchedEffect(job.id, reload) {
        loading = true
        var task: String? = null
        var ended = false
        try {
            val id = withContext(NonCancellable) { SyncApi.versions(job.id) }
            task = id
            val end = SyncApi.awaitTask(id)
            ended = true
            if (end.state == "done") {
                rows = end.resultAs<SyncVersions>()?.items ?: throw IllegalStateException("Versionsantwort ist nicht lesbar")
                error = null
            } else { error = end.failureText() }
        } catch (e: Exception) {
            if (e is kotlinx.coroutines.CancellationException) throw e
            error = e.displayText()
        } finally {
            val pending = task
            if (!ended && pending != null) withContext(NonCancellable) { runCatching { SyncApi.cancelTask(pending) } }
            loading = false
        }
    }
    Scaffold(topBar = {
        TopAppBar(title = { Text("Versionen: ${job.name}") }, navigationIcon = {
            IconButton(onClick = onBack) { SeIcon(R.drawable.ic_arrow_back, "Zurück") }
        }, actions = { IconButton(onClick = { reload++ }, enabled = !loading) { SeIcon(R.drawable.ic_refresh, "Aktualisieren") } })
    }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            LoadingBar(loading)
            LazyColumn(contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                error?.let { item { ErrorText(it) } }
                if (!loading && rows.isEmpty() && error == null) item { Text("Keine gesicherten Versionen für diesen Job.") }
                items(rows, key = { it.token }) { version ->
                    Card {
                        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(version.path, style = MaterialTheme.typography.titleSmall)
                            HintText("${BackgroundText.moment(version.preservedMs)} · ${version.size} Bytes · ${when (version.side) { "a" -> "Quelle"; "b" -> "Ziel"; else -> "Ältere Sicherung" }}")
                            OutlinedButton(onClick = { selected = version }, enabled = !loading) { Text("Wiederherstellen") }
                        }
                    }
                }
            }
        }
    }
    selected?.let { version ->
        AlertDialog(onDismissRequest = { selected = null }, title = { Text("Version wiederherstellen?") }, text = {
            Column {
                Text("${version.path}\n${BackgroundText.moment(version.preservedMs)}\n\nDie aktuelle Datei wird vorher gesichert. Der nächste Sync übernimmt die wiederhergestellte Version auf die andere Seite.")
                if (version.side == null) RadioGroup(listOf(SyncChoice("a", "In Quelle wiederherstellen"), SyncChoice("b", "In Ziel wiederherstellen")), legacySide, { legacySide = it })
            }
        }, confirmButton = { TextButton(onClick = {
            selected = null
            scope.launch {
                loading = true
                var task: String? = null
                var ended = false
                try {
                    val id = withContext(NonCancellable) { SyncApi.restoreVersion(job.id, version, version.side ?: legacySide) }
                    task = id
                    val end = SyncApi.awaitTask(id)
                    ended = true
                    error = if (end.state == "done") null else end.failureText()
                    if (error == null) reload++
                } catch (e: Exception) {
                    if (e is kotlinx.coroutines.CancellationException) throw e
                    error = e.displayText()
                } finally {
                    val pending = task
                    if (!ended && pending != null) withContext(NonCancellable) { runCatching { SyncApi.cancelTask(pending) } }
                    loading = false
                }
            }
        }) { Text("Wiederherstellen") } }, dismissButton = { TextButton(onClick = { selected = null }) { Text("Abbrechen") } })
    }
}
