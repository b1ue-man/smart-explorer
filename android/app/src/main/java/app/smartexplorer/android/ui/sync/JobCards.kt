// material3 1.4.0: several components may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.sync

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.SyncJob
import app.smartexplorer.android.api.SyncLastResult
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.core.VolumeInfo
import app.smartexplorer.android.service.BackgroundText
import app.smartexplorer.android.ui.common.SeIcon

/** Header of the Sync page: background status, tap opens the background settings (spec F15). */
@Composable
internal fun BackgroundHeaderCard(summary: String, onOpen: () -> Unit) {
    Card(onClick = onOpen, modifier = Modifier.fillMaxWidth()) {
        Row(
            Modifier.padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SeIcon(R.drawable.ic_sync, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
            Column(Modifier.weight(1f)) {
                Text("Hintergrund", style = MaterialTheme.typography.labelLarge)
                Text(summary, style = MaterialTheme.typography.bodyMedium)
            }
            SeIcon(R.drawable.ic_chevron_right, contentDescription = null)
        }
    }
}

/** "Noch keine Sync-Jobs" with [Job anlegen] [Ordner spiegeln]. */
@Composable
internal fun NoJobs(onCreate: () -> Unit, onMirror: () -> Unit) {
    Column(
        Modifier.fillMaxWidth().padding(vertical = 48.dp, horizontal = 16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        SeIcon(R.drawable.ic_sync, contentDescription = null, modifier = Modifier.size(48.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Text("Noch keine Sync-Jobs", style = MaterialTheme.typography.titleMedium)
        Text(
            "Ein Job hält zwei Orte abgeglichen – lokal, remote oder auf einem Share-Gerät.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = onCreate) { Text("Job anlegen") }
            OutlinedButton(onClick = onMirror) { Text("Ordner spiegeln") }
        }
    }
}

/** Actions of one job card. */
internal class JobActions(
    val onRun: () -> Unit,
    val onEdit: () -> Unit,
    val onToggle: () -> Unit,
    val onConflicts: () -> Unit,
    val onDelete: () -> Unit,
)

/**
 * Job card (spec F15): name, "A ⇄ B" with place symbols, trigger, last run (✓ / ⚠ n Konflikte /
 * ✕ Fehler) or a bar while it runs; ▶ runs it now; ⋮ edit, enable/pause, conflicts, delete.
 * [runTask] is the facade task of a "Jetzt" run; [daemonRuns] marks a run of the daemon.
 */
@Composable
internal fun JobCard(
    job: SyncJob,
    volumes: List<VolumeInfo>,
    running: Boolean,
    runTask: TaskInfo?,
    daemonRuns: Boolean,
    actions: JobActions,
) {
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(start = 16.dp, end = 4.dp, top = 8.dp, bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(job.name.ifBlank { "Ohne Namen" }, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(
                        if (job.enabled) job.schedule.ifBlank { "Manuell" } else "Pausiert · ${job.schedule.ifBlank { "Manuell" }}",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                IconButton(onClick = actions.onRun, enabled = !running && !daemonRuns) {
                    SeIcon(R.drawable.ic_play, contentDescription = "Jetzt ausführen")
                }
                JobMenu(job, actions)
            }
            Endpoint(null, job.source, volumes)
            Endpoint(SyncLocations.arrow(job.direction), job.target, volumes)
            Box(Modifier.padding(end = 12.dp)) {
                when {
                    running -> RunningLine(runTask?.message?.ifBlank { null } ?: "Synchronisiere…")
                    daemonRuns -> RunningLine("Läuft im Hintergrund…")
                    else -> LastResultLine(job.lastResult, actions.onConflicts)
                }
            }
        }
    }
}

@Composable
private fun JobMenu(job: SyncJob, actions: JobActions) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { SeIcon(R.drawable.ic_more_vert, contentDescription = "Weitere Aktionen") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            MenuEntry("Bearbeiten", { open = false }, actions.onEdit)
            MenuEntry(if (job.enabled) "Pausieren" else "Aktivieren", { open = false }, actions.onToggle)
            if (job.direction == SyncJob.DIRECTION_BOTH) MenuEntry("Konflikte", { open = false }, actions.onConflicts)
            MenuEntry("Löschen", { open = false }, actions.onDelete)
        }
    }
}

@Composable
private fun MenuEntry(label: String, close: () -> Unit, action: () -> Unit) {
    DropdownMenuItem(
        text = { Text(label) },
        onClick = {
            close()
            action()
        },
    )
}

@Composable
private fun Endpoint(arrow: String?, location: String, volumes: List<VolumeInfo>) {
    Row(
        Modifier.padding(end = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(arrow ?: " ", style = MaterialTheme.typography.bodyMedium, modifier = Modifier.size(width = 16.dp, height = 20.dp))
        SeIcon(SyncLocations.icon(location), contentDescription = null, modifier = Modifier.size(18.dp))
        Text(SyncLocations.label(location, volumes), style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun RunningLine(text: String) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(text, style = MaterialTheme.typography.bodySmall)
        LinearProgressIndicator(Modifier.fillMaxWidth())
    }
}

@Composable
private fun LastResultLine(result: SyncLastResult?, onConflicts: () -> Unit) {
    if (result == null || result.timeMs <= 0) {
        Text("Noch nicht gelaufen", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        return
    }
    val time = BackgroundText.moment(result.timeMs)
    Column {
        when {
            result.conflicts > 0 -> AssistChip(
                onClick = onConflicts,
                label = { Text("⚠ ${result.conflicts} Konflikte · $time") },
                leadingIcon = { SeIcon(R.drawable.ic_warning, contentDescription = null, modifier = Modifier.size(18.dp)) },
            )
            result.errors > 0 -> Text(
                "✕ $time · ${result.errors} Fehler",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
            else -> Text(
                "✓ $time · A→B ${result.aToB}, B→A ${result.bToA}, gelöscht ${result.deleted}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (result.note.isNotBlank()) {
            Text(result.note, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
    }
}
