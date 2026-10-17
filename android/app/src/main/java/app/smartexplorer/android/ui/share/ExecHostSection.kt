package app.smartexplorer.android.ui.share

import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import app.smartexplorer.android.R
import app.smartexplorer.android.api.AccessApproval
import app.smartexplorer.android.api.AccessChange
import app.smartexplorer.android.api.AccessOverview
import app.smartexplorer.android.api.AccessView
import app.smartexplorer.android.api.EXEC_INCOMING
import app.smartexplorer.android.api.EXEC_RELATION_ROOM
import app.smartexplorer.android.api.ExecJob
import app.smartexplorer.android.api.ExecJobs
import app.smartexplorer.android.api.ExecProviderInfo
import app.smartexplorer.android.api.ExecTarget
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.service.AccessApprovalNotifier
import app.smartexplorer.android.system.Permissions
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SectionHeader

/** Recent finished commands shown below the running ones. */
private const val RECENT_JOBS = 10

/** Callbacks of the section "Befehle auf diesem Telefon". */
internal class ExecHostActions(
    /** `share.setExec` enabling; the access dialog ([setAccess]) replaced its warning, callers still pass it. */
    val allow: (ExecTarget) -> Unit,
    val revoke: (ExecTarget) -> Unit,
    val stop: (ExecJob) -> Unit,
    /** „Programme…“: the program list of one device. */
    val programs: (ExecTarget) -> Unit,
    /** „Befehlsprotokoll“ of this phone. */
    val log: () -> Unit,
    /** Saves the access dialog of one device; `null`: through the Share view model (`share.setAccess`). */
    val setAccess: ((ExecTarget, AccessChange) -> Unit)? = null,
)

/**
 * Section "Befehle auf diesem Telefon" (spec G4 B, plan 2026-10-17 PRM-M10): per Direct device and
 * room member (no global switch) [Erlauben…] opens the access dialog (commands and terminal each
 * „Aus“, „Fragen“ or „Erlauben“, „Zeitraum“, programs, „Ausführen als: App-Konto“), ⋮ „Zugriff…“
 * changes it, [Entziehen] turns both off without a question, ⋮ „Programme…“ narrows a device to a
 * program list. Each row names its right, the account („Standardnutzer“: Android runs commands only
 * as the app) and the programs. Open questions of other devices are listed with [Antworten…]; a
 * right on „Fragen“ while notifications are off shows a warning with „Benachrichtigungen erlauben“.
 * Below, the running commands of other devices with [Stopp], the last finished ones and
 * [Befehlsprotokoll]. Without an exec provider the reason is shown and allowing is disabled.
 * [access] defaults to the Share view model's access fields.
 */
@Composable
internal fun ExecHostSection(
    status: ShareStatus,
    jobs: ExecJobs?,
    actions: ExecHostActions,
    access: AccessOverview? = null,
) {
    val vm: ShareViewModel? = if (access == null || actions.setAccess == null) viewModel { ShareViewModel() } else null
    val overview = access ?: vm?.access
    val saveAccess: (ExecTarget, AccessChange) -> Unit = actions.setAccess ?: { target, change -> vm?.setAccess(target, change) }
    val provider = status.execProvider
    val pending by AccessApprovalNotifier.pending.collectAsStateWithLifecycle()
    // Keys, not targets: a lazy item keeps saveable state while scrolled away.
    var accessKey by rememberSaveable { mutableStateOf<String?>(null) }
    var answerId by rememberSaveable { mutableStateOf<String?>(null) }
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
    if (overview?.execTargets.orEmpty().any { it.access.asks() }) NotificationWarning()
    if (pending.isNotEmpty()) {
        JobsTitle("Offene Rückfragen")
        pending.forEach { approval -> ApprovalRow(approval, onAnswer = { answerId = approval.id }) }
    }
    if (status.execTargets.isEmpty()) HintLine("Noch keine Direkt-Geräte oder Raummitglieder.")
    status.execTargets.forEach { target ->
        ExecTargetRow(
            target,
            provider,
            overview?.accessOf(target.targetKey),
            onAccess = { accessKey = target.targetKey },
            onRevoke = { actions.revoke(target) },
            onPrograms = { actions.programs(target) },
        )
    }
    TextButton(onClick = actions.log, modifier = Modifier.padding(start = 4.dp)) { Text("Befehlsprotokoll anzeigen") }
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

    val accessTarget = accessKey?.let { key -> status.execTargets.firstOrNull { it.targetKey == key } }
    // The target vanished meanwhile: forget the open dialog.
    if (accessKey != null && accessTarget == null) LaunchedEffect(accessKey) { accessKey = null }
    accessTarget?.let { target ->
        AccessDialog(
            target,
            overview?.accessOf(target.targetKey),
            overview?.execProvider,
            onConfirm = { change ->
                accessKey = null
                saveAccess(target, change)
            },
            onDismiss = { accessKey = null },
        )
    }
    val answer = answerId?.let { id -> pending.firstOrNull { it.id == id } }
    // Answered elsewhere or withdrawn: nothing left to answer.
    if (answerId != null && answer == null) LaunchedEffect(answerId) { answerId = null }
    answer?.let { approval ->
        AccessApprovalPrompt(
            approval,
            onDecide = { decision ->
                answerId = null
                AccessApprovalNotifier.decide(approval.id, decision)
            },
            onDismiss = { answerId = null },
        )
    }
}

@Composable
private fun JobsTitle(text: String) {
    Text(text, style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(start = 16.dp, top = 12.dp))
}

@Composable
private fun ExecTargetRow(
    target: ExecTarget,
    provider: ExecProviderInfo,
    view: AccessView?,
    onAccess: () -> Unit,
    onRevoke: () -> Unit,
    onPrograms: () -> Unit,
) {
    val state = buildString {
        append(execRelationText(target))
        when {
            view != null -> append(" · ").append(accessSummary(view))
            target.enabled -> append(" · Befehle erlaubt")
            else -> append(" · Keine Befehle")
        }
        if (!target.baseAuthorized) append(" · Freigabe inaktiv")
        append("\nAusführen als: ").append(execRunAsText(target, provider))
        append("\nProgramme: ").append(execProgramsText(target.programs))
    }
    ListItem(
        headlineContent = { Text(target.name.ifBlank { "Gerät" }, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(state, maxLines = 4, overflow = TextOverflow.Ellipsis) },
        leadingContent = {
            SeIcon(
                R.drawable.ic_terminal,
                contentDescription = null,
                tint = if (target.enabled) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        trailingContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (target.enabled) {
                    TextButton(onClick = onRevoke) { Text("Entziehen") }
                } else {
                    TextButton(onClick = onAccess, enabled = provider.available && target.baseAuthorized) { Text("Erlauben…") }
                }
                RowMenu(listOf("Zugriff…" to onAccess, "Programme…" to onPrograms))
            }
        },
    )
}

@Composable
private fun ApprovalRow(approval: AccessApproval, onAnswer: () -> Unit) {
    ListItem(
        headlineContent = { Text(approvalTitle(approval), maxLines = 2, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(approvalLines(approval).joinToString("\n"), maxLines = 4, overflow = TextOverflow.Ellipsis) },
        leadingContent = { SeIcon(R.drawable.ic_lock, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
        trailingContent = { TextButton(onClick = onAnswer) { Text("Antworten…") } },
    )
}

/**
 * A right on „Fragen“ while notifications are off (plan C5): a question can then only show while the
 * app is open; otherwise the other device is refused with `approval_unavailable`.
 */
@Composable
private fun NotificationWarning() {
    val context = LocalContext.current
    var allowed by remember { mutableStateOf(Permissions.canPostNotifications(context)) }
    var asked by rememberSaveable { mutableStateOf(false) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { allowed = Permissions.canPostNotifications(context) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        allowed = Permissions.canPostNotifications(context)
    }
    if (allowed) return
    Text(
        "Benachrichtigungen sind aus: Rückfragen („Fragen“) erscheinen nur, solange die App geöffnet ist; " +
            "sonst wird das andere Gerät abgewiesen.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.error,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 2.dp),
    )
    TextButton(
        onClick = {
            if (Permissions.notificationsNeedRuntimePermission() && !asked) {
                asked = true
                launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
            } else if (!Permissions.openNotificationSettings(context)) {
                Snackbars.show("Einstellung nicht verfügbar – bitte in den App-Einstellungen erlauben.")
            }
        },
        modifier = Modifier.padding(start = 4.dp),
    ) { Text("Benachrichtigungen erlauben") }
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
