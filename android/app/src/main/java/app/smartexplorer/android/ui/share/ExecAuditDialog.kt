package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.ExecAuditEntry
import app.smartexplorer.android.api.ShareExecApi
import app.smartexplorer.android.core.CoreException
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/** Entries the dialog loads (the core caps at its own maximum). */
private const val LOG_LINES = 200

private val LOG_TIME = DateTimeFormatter.ofPattern("dd.MM.yyyy HH:mm:ss")

/** Local time of an entry's RFC 3339 [at]; the text itself when it does not parse. */
internal fun execAuditTime(at: String): String = try {
    OffsetDateTime.parse(at).atZoneSameInstant(ZoneId.systemDefault()).format(LOG_TIME)
} catch (e: java.time.format.DateTimeParseException) {
    at.ifBlank { "–" }
}

/** Entries that stand out: refusals, and confirmations that ended without a yes. */
private val EXEC_AUDIT_REFUSALS = setOf(
    "auth_rejected",
    "penalty_refused",
    "start_refused",
    "unreadable",
    "approval_denied",
    "approval_cancelled",
    "approval_unavailable",
    "grant_expired",
    "helper_refused",
)

/** Refusals stand out from starts and ends. */
internal fun execAuditIsRefusal(entry: ExecAuditEntry): Boolean = entry.event in EXEC_AUDIT_REFUSALS

/**
 * „Befehlsprotokoll“ of this phone (`share.execLog`, as `se share exec log` and the desktop page
 * „Befehle“): refused logins and starts, starts and ends, confirmations and ended grants, newest
 * first; each entry names kind, account and confirmation in its detail line. Shell command text is
 * never recorded; text of the other device is cut and cleaned by the core.
 */
@Composable
internal fun ExecAuditDialog(onDismiss: () -> Unit) {
    var entries by remember { mutableStateOf<List<ExecAuditEntry>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var reload by remember { mutableIntStateOf(0) }
    LaunchedEffect(reload) {
        error = null
        try {
            entries = ShareExecApi.log(LOG_LINES).entries
        } catch (e: CoreException) {
            error = e.message ?: e.kind
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Befehlsprotokoll") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    "Abgelehnte Anmeldungen und Starts, Starts und Enden von Befehlen anderer Geräte auf diesem Telefon, " +
                        "Rückfragen mit ihrer Antwort und abgelaufene Rechte. Shell-Befehle werden nie im Wortlaut gespeichert.",
                    style = MaterialTheme.typography.bodySmall,
                )
                error?.let { Text("Protokoll nicht lesbar: $it", color = MaterialTheme.colorScheme.error) }
                val shown = entries
                when {
                    shown == null && error == null -> LinearProgressIndicator(Modifier.fillMaxWidth())
                    shown != null && shown.isEmpty() -> Text("Noch keine Einträge.")
                    shown != null -> LazyColumn(Modifier.heightIn(max = 420.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        items(shown) { entry -> ExecAuditRow(entry) }
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Schließen") } },
        dismissButton = { TextButton(onClick = { reload++ }) { Text("Neu laden") } },
    )
}

@Composable
private fun ExecAuditRow(entry: ExecAuditEntry) {
    Column {
        Text(
            "${execAuditTime(entry.at)} · ${entry.title}",
            style = MaterialTheme.typography.labelLarge,
            color = if (execAuditIsRefusal(entry)) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
        )
        if (entry.device.isNotBlank()) Text(entry.device, style = MaterialTheme.typography.bodySmall)
        if (entry.detail.isNotBlank()) {
            Text(entry.detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}
