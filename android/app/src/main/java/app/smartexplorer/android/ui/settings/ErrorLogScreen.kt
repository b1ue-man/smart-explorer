package app.smartexplorer.android.ui.settings

import android.os.Build
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.BuildConfig
import app.smartexplorer.android.R
import app.smartexplorer.android.api.ErrorLogEntry
import app.smartexplorer.android.api.SysApi
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.ConfirmDialog
import app.smartexplorer.android.ui.common.EmptyState
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.more.TextActions
import app.smartexplorer.android.ui.more.TextReportDialog
import kotlinx.coroutines.launch

/**
 * Error log (spec F23): app errors (time, action, message), newest first, the core's crash log as
 * its own entry; [Kopieren] [Teilen] [Leeren]. New `error` events reload the list.
 */
@Composable
fun ErrorLogScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var entries by remember { mutableStateOf<List<ErrorLogEntry>?>(null) }
    var crashLog by remember { mutableStateOf("") }
    var loadError by remember { mutableStateOf<String?>(null) }
    var reloadKey by remember { mutableIntStateOf(0) }
    var confirmClear by remember { mutableStateOf(false) }
    var detail by remember { mutableStateOf<Pair<String, String>?>(null) }

    LaunchedEffect(reloadKey) {
        try {
            entries = SysApi.errors().sortedByDescending { it.timeMs }
            crashLog = SysApi.crashLog()
            loadError = null
        } catch (e: CoreException) {
            loadError = e.message ?: e.kind
        }
    }
    LaunchedEffect(Unit) {
        Core.events.collect { if (it is CoreEvent.Error) reloadKey++ }
    }

    val list = entries
    SubPageScaffold(
        title = "Fehlerprotokoll",
        onBack = onBack,
        actions = {
            IconButton(onClick = { TextActions.copy(context, "Fehlerprotokoll", exportText(list, crashLog)) }, enabled = list != null) {
                SeIcon(R.drawable.ic_copy, contentDescription = "Kopieren")
            }
            IconButton(
                onClick = { TextActions.share(context, "Smart Explorer – Fehlerprotokoll", exportText(list, crashLog)) },
                enabled = list != null,
            ) { SeIcon(R.drawable.ic_share, contentDescription = "Teilen") }
            IconButton(onClick = { confirmClear = true }, enabled = !list.isNullOrEmpty()) {
                SeIcon(R.drawable.ic_delete, contentDescription = "Leeren")
            }
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            LoadingBar(list == null && loadError == null)
            val error = loadError
            when {
                error != null -> ErrorCard(
                    error,
                    modifier = Modifier.padding(16.dp),
                    title = "Protokoll nicht geladen",
                    actionLabel = "Erneut",
                    onAction = { reloadKey++ },
                )
                list == null -> Unit
                list.isEmpty() && crashLog.isBlank() -> EmptyState(
                    R.drawable.ic_check,
                    "Keine Fehler protokolliert",
                    message = "Fehler aus Vorgängen und Hintergrundläufen erscheinen hier.",
                )
                else -> LazyColumn(Modifier.fillMaxSize()) {
                    if (crashLog.isNotBlank()) {
                        item(key = "crash") {
                            ListItem(
                                overlineContent = { Text("Absturzprotokoll des Kerns") },
                                headlineContent = { Text(crashLog.lineSequence().firstOrNull().orEmpty(), maxLines = 2, overflow = TextOverflow.Ellipsis) },
                                leadingContent = { SeIcon(R.drawable.ic_error, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
                                modifier = Modifier.clickable { detail = "Absturzprotokoll" to crashLog },
                            )
                        }
                    }
                    itemsIndexed(list, key = { index, _ -> index }) { _, entry ->
                        ListItem(
                            overlineContent = { Text("${Format.dateTime(entry.timeMs)} · ${entry.action}") },
                            headlineContent = { Text(entry.message, maxLines = 3, overflow = TextOverflow.Ellipsis) },
                            leadingContent = { SeIcon(R.drawable.ic_warning, contentDescription = null) },
                            modifier = Modifier.clickable { detail = entry.action.ifBlank { "Fehler" } to entry.message },
                        )
                    }
                }
            }
        }
    }

    detail?.let { (title, text) -> TextReportDialog(title, text, onDismiss = { detail = null }) }
    if (confirmClear) {
        ConfirmDialog(
            title = "Fehlerprotokoll leeren?",
            message = "Alle Einträge werden gelöscht. Das Absturzprotokoll des Kerns bleibt erhalten.",
            confirmLabel = "Leeren",
            destructive = true,
            onConfirm = {
                confirmClear = false
                scope.launch {
                    try {
                        SysApi.clearErrors()
                    } catch (e: CoreException) {
                        Snackbars.show("Nicht geleert: ${e.message ?: e.kind}")
                    }
                    reloadKey++
                }
            },
            onDismiss = { confirmClear = false },
        )
    }
}

/** Plain text for [Kopieren]/[Teilen]: app, core and device, then crash log and entries. */
private fun exportText(entries: List<ErrorLogEntry>?, crashLog: String): String = buildString {
    appendLine("Smart Explorer ${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})")
    appendLine("Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT}), ${Build.MANUFACTURER} ${Build.MODEL}")
    if (crashLog.isNotBlank()) {
        appendLine()
        appendLine("Absturzprotokoll des Kerns:")
        appendLine(crashLog.trimEnd())
    }
    appendLine()
    if (entries.isNullOrEmpty()) appendLine("Keine Fehler protokolliert.")
    entries.orEmpty().forEach { appendLine("${Format.dateTime(it.timeMs)}  ${it.action}: ${it.message}") }
}
