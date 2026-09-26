package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.EXEC_INCOMING
import app.smartexplorer.android.api.EXEC_RELATION_ROOM
import app.smartexplorer.android.api.ExecJob
import app.smartexplorer.android.api.ExecJobs
import app.smartexplorer.android.api.ExecTarget
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SectionHeader

/** Recent finished commands shown below the running ones. */
private const val RECENT_JOBS = 10

/** Callbacks of the section "Befehle auf diesem Telefon". */
internal class ExecHostActions(
    /** Called after the warning was confirmed. */
    val allow: (ExecTarget) -> Unit,
    val revoke: (ExecTarget) -> Unit,
    val stop: (ExecJob) -> Unit,
)

/**
 * Section "Befehle auf diesem Telefon" (spec G4 B): per Direct device and room member (no global
 * switch) [Erlauben…] → warning with the Android risks and "Ich habe verstanden" → [Aktivieren];
 * [Entziehen] without a question. Below, the running commands of other devices with [Stopp] and
 * the last finished ones. Without an exec provider the reason is shown and allowing is disabled.
 */
@Composable
internal fun ExecHostSection(status: ShareStatus, jobs: ExecJobs?, actions: ExecHostActions) {
    val provider = status.execProvider
    // The key, not the target: a lazy item keeps saveable state while scrolled away.
    var confirmKey by rememberSaveable { mutableStateOf<String?>(null) }
    SectionHeader("Befehle auf diesem Telefon")
    HintLine("Andere Geräte dürfen hier nur nach deiner ausdrücklichen Erlaubnis Befehle ausführen – je Gerät bzw. Raummitglied.")
    if (!provider.available) {
        Text(
            "Nicht verfügbar: ${provider.detail.ifBlank { provider.provider }}",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.error,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 2.dp),
        )
    }
    if (status.execTargets.isEmpty()) HintLine("Noch keine Direkt-Geräte oder Raummitglieder.")
    status.execTargets.forEach { target ->
        ExecTargetRow(target, provider.available, onAllow = { confirmKey = target.targetKey }, onRevoke = { actions.revoke(target) })
    }
    val running = runningIncoming(jobs)
    if (running.isNotEmpty()) {
        JobsTitle("Laufende Befehle")
        running.forEach { job -> ExecJobRow(job, onStop = { actions.stop(job) }) }
    }
    val recent = recentIncoming(jobs, RECENT_JOBS)
    if (recent.isNotEmpty()) {
        JobsTitle("Letzte Befehle")
        recent.forEach { job -> ExecJobRow(job, onStop = null) }
    }

    val confirm = confirmKey?.let { key -> status.execTargets.firstOrNull { it.targetKey == key && !it.enabled } }
    // The target vanished or was allowed meanwhile: forget the open question.
    if (confirmKey != null && confirm == null) LaunchedEffect(confirmKey) { confirmKey = null }
    confirm?.let { target ->
        ExecGrantDialog(
            target,
            onConfirm = {
                confirmKey = null
                actions.allow(target)
            },
            onDismiss = { confirmKey = null },
        )
    }
}

@Composable
private fun JobsTitle(text: String) {
    Text(text, style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(start = 16.dp, top = 12.dp))
}

@Composable
private fun ExecTargetRow(target: ExecTarget, providerAvailable: Boolean, onAllow: () -> Unit, onRevoke: () -> Unit) {
    val state = buildString {
        append(execRelationText(target))
        append(if (target.enabled) " · Befehle erlaubt" else " · Keine Befehle")
        if (!target.baseAuthorized) append(" · Freigabe inaktiv")
    }
    ListItem(
        headlineContent = { Text(target.name.ifBlank { "Gerät" }, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(state, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        leadingContent = {
            SeIcon(
                R.drawable.ic_terminal,
                contentDescription = null,
                tint = if (target.enabled) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        trailingContent = {
            if (target.enabled) {
                TextButton(onClick = onRevoke) { Text("Entziehen") }
            } else {
                TextButton(onClick = onAllow, enabled = providerAvailable && target.baseAuthorized) { Text("Erlauben…") }
            }
        },
    )
}

@Composable
private fun ExecJobRow(job: ExecJob, onStop: (() -> Unit)?) {
    ListItem(
        headlineContent = { Text(execJobTitle(job), maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(execJobLine(job), maxLines = 3, overflow = TextOverflow.Ellipsis) },
        trailingContent = if (onStop != null) {
            { TextButton(onClick = onStop) { Text("Stopp") } }
        } else {
            null
        },
    )
}

/** The warning before a device may run commands here; [Aktivieren] only after "Ich habe verstanden". */
@Composable
private fun ExecGrantDialog(target: ExecTarget, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    var understood by rememberSaveable(target.targetKey) { mutableStateOf(false) }
    val name = target.name.ifBlank { "Dieses Gerät" }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Befehle von „$name“ erlauben?") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("„$name“ kann danach ohne weitere Rückfrage beliebige Shell-Befehle auf diesem Telefon starten.")
                Text(
                    "Die Befehle laufen mit den Rechten von Smart Explorer: Sie können die Daten der App lesen – " +
                        "gespeicherte Verbindungspasswörter, die Share-Identität und Google-Drive-Tokens – und alle " +
                        "Dateien, auf die die App zugreifen darf.",
                    color = MaterialTheme.colorScheme.error,
                )
                Text("Android kann Befehle im Hintergrund beenden oder anhalten; mit der App enden auch laufende Befehle.")
                Text(
                    "Die Erlaubnis gilt nur für genau diese Geräte-Identität (${execRelationText(target)}, " +
                        "Fingerabdruck ${target.fingerprint}) und lässt sich jederzeit entziehen.",
                    style = MaterialTheme.typography.bodySmall,
                )
                Row(
                    Modifier.toggleable(value = understood, role = Role.Checkbox, onValueChange = { understood = it }),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Checkbox(checked = understood, onCheckedChange = null)
                    Text("Ich habe verstanden", modifier = Modifier.padding(start = 8.dp))
                }
            }
        },
        confirmButton = { TextButton(onClick = onConfirm, enabled = understood) { Text("Aktivieren") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } },
    )
}

/** "Direkt-Gerät" or "Raum „Team“". */
internal fun execRelationText(target: ExecTarget): String =
    if (target.relation == EXEC_RELATION_ROOM) "Raum „${target.roomName.orEmpty().ifBlank { "Raum" }}“" else "Direkt-Gerät"

/** Running commands other devices started on this phone. */
internal fun runningIncoming(jobs: ExecJobs?): List<ExecJob> = jobs?.active.orEmpty().filter { it.direction == EXEC_INCOMING }

/** The last [limit] finished commands of other devices, newest first. */
internal fun recentIncoming(jobs: ExecJobs?, limit: Int): List<ExecJob> =
    jobs?.history.orEmpty()
        .filter { it.direction == EXEC_INCOMING }
        .sortedByDescending { it.finishedAt ?: it.startedAt ?: 0L }
        .take(limit)

/** "Laptop: Shell-Befehl" (a shell command's program is `<shell>`). */
internal fun execJobTitle(job: ExecJob): String {
    val program = if (job.program == "<shell>" || job.program.isBlank()) "Shell-Befehl" else job.program
    return "${job.peerName.ifBlank { "Gerät" }}: $program"
}

/** State, exit code, start time and message of a command. */
internal fun execJobLine(job: ExecJob): String = buildList {
    add(execStateLabel(job.state) + (job.exitCode?.let { " (Code $it)" } ?: ""))
    job.startedAt?.let { add("gestartet ${Format.dateTime(it * 1000)}") }
    job.message?.takeIf { it.isNotBlank() }?.let { add(it) }
}.joinToString(" · ")

/** German label of a lifecycle state (`ExecJob.state`). */
internal fun execStateLabel(state: String): String = when (state) {
    "queued_local" -> "Wartet"
    "connecting" -> "Verbindet"
    "authenticating" -> "Anmeldung"
    "authorized" -> "Erlaubt"
    "starting" -> "Startet"
    "running" -> "Läuft"
    "cancelling" -> "Wird gestoppt"
    "exited" -> "Beendet"
    "failed" -> "Fehlgeschlagen"
    "timed_out" -> "Zeitlimit erreicht"
    "cancelled" -> "Gestoppt"
    "revoked" -> "Erlaubnis entzogen"
    "disconnected" -> "Verbindung getrennt"
    else -> state.ifBlank { "Unbekannt" }
}
