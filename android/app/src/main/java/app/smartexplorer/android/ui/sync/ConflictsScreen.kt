// material3 1.4.0: several components may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.sync

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.R
import app.smartexplorer.android.api.ConflictSide
import app.smartexplorer.android.api.SyncConflict
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.ui.common.EmptyState
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon

/**
 * Conflict list (spec F16): file, times/sizes of A and B (missing = deleted), per entry
 * [A behalten] [B behalten] [Überspringen] and [Zusammenführen] for text; [Alle A] [Alle B] on top.
 */
@Composable
internal fun ConflictsScreen(
    session: ConflictSession,
    onBack: () -> Unit,
    onAbandon: () -> Unit,
    onMerge: (SyncConflict) -> Unit,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text("Konflikte", maxLines = 1)
                        Text(
                            session.job.name,
                            style = MaterialTheme.typography.bodySmall,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                },
                navigationIcon = {
                    IconButton(onClick = onBack) { SeIcon(R.drawable.ic_arrow_back, contentDescription = "Zurück") }
                },
                actions = {
                    IconButton(onClick = session::check, enabled = !session.working) {
                        SeIcon(R.drawable.ic_refresh, contentDescription = "Konflikte prüfen")
                    }
                },
            )
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            LoadingBar(session.loading || session.bulk || session.finishing)
            session.finishError?.let { error ->
                ErrorCard(
                    message = "$error\nOhne Speichern erkennt der nächste Lauf dieselben Konflikte erneut.",
                    modifier = Modifier.padding(16.dp),
                    title = "Auflösungen nicht gespeichert",
                    actionLabel = "Erneut versuchen",
                    onAction = onBack,
                )
                TextButton(onClick = onAbandon, modifier = Modifier.padding(horizontal = 16.dp)) { Text("Ohne Speichern schließen") }
            }
            session.checkTaskId?.let { CheckProgress(it, onCancel = session::cancelCheck) }
            val loadError = session.loadError
            when {
                loadError != null && session.items.isEmpty() -> ErrorCard(
                    message = loadError,
                    modifier = Modifier.padding(16.dp),
                    title = "Konflikte nicht geladen",
                    actionLabel = "Erneut laden",
                    onAction = session::load,
                )
                !session.available && session.checkTaskId == null && !session.loading -> EmptyState(
                    icon = R.drawable.ic_warning,
                    title = "Konfliktliste nicht geladen",
                    message = "Nach einem Neustart der App oder einem Hintergrundlauf ermittelt ein Probelauf die Konflikte.",
                    actionLabel = "Konflikte prüfen",
                    onAction = session::check,
                )
                session.available && session.items.isEmpty() && !session.loading -> EmptyState(
                    icon = R.drawable.ic_check,
                    title = "Keine offenen Konflikte",
                )
                else -> ConflictList(session, onMerge)
            }
        }
    }
}

@Composable
private fun ConflictList(session: ConflictSession, onMerge: (SyncConflict) -> Unit) {
    Column(Modifier.fillMaxSize()) {
        if (session.items.isNotEmpty()) {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("${session.items.size} offen", style = MaterialTheme.typography.labelLarge, modifier = Modifier.weight(1f))
                OutlinedButton(onClick = { session.resolveAll("a") }, enabled = !session.working) { Text("Alle A") }
                OutlinedButton(onClick = { session.resolveAll("b") }, enabled = !session.working) { Text("Alle B") }
            }
        }
        LazyColumn(
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            items(session.items, key = { it.key }) { item ->
                ConflictCard(
                    item = item,
                    busy = session.busy[item.key] == true,
                    enabled = !session.bulk && !session.finishing,
                    onKeepA = { session.resolve(item, "a") },
                    onKeepB = { session.resolve(item, "b") },
                    onSkip = { session.skip(item) },
                    onMerge = { onMerge(item) },
                )
            }
        }
    }
}

@Composable
private fun ConflictCard(
    item: SyncConflict,
    busy: Boolean,
    enabled: Boolean,
    onKeepA: () -> Unit,
    onKeepB: () -> Unit,
    onSkip: () -> Unit,
    onMerge: () -> Unit,
) {
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(item.path, style = MaterialTheme.typography.titleSmall, maxLines = 3, overflow = TextOverflow.Ellipsis)
            Text("A: ${sideText(item.a)}", style = MaterialTheme.typography.bodyMedium)
            Text("B: ${sideText(item.b)}", style = MaterialTheme.typography.bodyMedium)
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth().padding(top = 4.dp))
        }
        val actionsEnabled = enabled && !busy
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp)) {
            TextButton(onClick = onKeepA, enabled = actionsEnabled) { Text("A behalten") }
            TextButton(onClick = onKeepB, enabled = actionsEnabled) { Text("B behalten") }
            TextButton(onClick = onSkip, enabled = actionsEnabled) { Text("Überspringen") }
            if (item.text) TextButton(onClick = onMerge, enabled = actionsEnabled) { Text("Zusammenführen") }
        }
    }
}

/** "3,2 MB · 12.09.2026 14:03", or "gelöscht" when the side is missing. */
private fun sideText(side: ConflictSide?): String =
    if (side == null || !side.exists) "gelöscht" else "${Format.size(side.size)} · ${Format.dateTime(side.mtimeMs)}"

/** Progress of the dry run with [Abbrechen]. */
@Composable
private fun CheckProgress(taskId: String, onCancel: () -> Unit) {
    val tasks by Core.tasks.collectAsStateWithLifecycle()
    val task = tasks.firstOrNull { it.id == taskId }
    Row(
        Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(task?.message?.ifBlank { null } ?: "Konflikte werden geprüft…", style = MaterialTheme.typography.bodyMedium)
            val fraction = task?.let { Format.fraction(it.doneItems, it.totalItems) }
            if (fraction != null) {
                LinearProgressIndicator(progress = { fraction }, modifier = Modifier.fillMaxWidth())
            } else {
                LinearProgressIndicator(Modifier.fillMaxWidth())
            }
        }
        TextButton(onClick = onCancel) { Text("Abbrechen") }
    }
}
