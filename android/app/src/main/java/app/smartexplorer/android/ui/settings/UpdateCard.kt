package app.smartexplorer.android.ui.settings

import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.BuildConfig
import app.smartexplorer.android.R
import app.smartexplorer.android.system.Permissions
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.rememberTask
import app.smartexplorer.android.update.UpdateChecker
import app.smartexplorer.android.update.UpdateInstaller
import app.smartexplorer.android.update.UpdateState

/**
 * Update card (spec F22): check, "Version x.y.z verfügbar [Installieren]", download progress with
 * [Abbrechen], the "Unbekannte Apps installieren" hint and failures such as a broken download.
 */
@Composable
fun UpdateCard(modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val state by UpdateChecker.state.collectAsStateWithLifecycle()
    val highlight = state is UpdateState.Available || state is UpdateState.Ready
    Card(
        modifier = modifier.fillMaxWidth(),
        colors = if (highlight) {
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.primaryContainer,
                contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
            )
        } else {
            CardDefaults.cardColors()
        },
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                SeIcon(R.drawable.ic_update, contentDescription = null)
                Text(title(state), style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            }
            when (val current = state) {
                UpdateState.Idle, is UpdateState.UpToDate -> {
                    Text("Installiert: Version ${BuildConfig.VERSION_NAME}", style = MaterialTheme.typography.bodyMedium)
                    OutlinedButton(onClick = { UpdateChecker.checkNow(context) }) { Text("Jetzt prüfen") }
                }
                UpdateState.Checking -> LoadingBar(loading = true)
                is UpdateState.Available -> {
                    current.info.notes?.takeIf { it.isNotBlank() }?.let { notes ->
                        Text(notes, style = MaterialTheme.typography.bodyMedium, maxLines = 6, overflow = TextOverflow.Ellipsis)
                    }
                    Button(onClick = { UpdateChecker.download(context) }) { Text("Installieren") }
                }
                is UpdateState.Downloading -> DownloadProgress(current.taskId)
                is UpdateState.Ready -> {
                    if (current.needsPermission) {
                        Text(
                            "Zum Installieren braucht Smart Explorer die Erlaubnis „Unbekannte Apps installieren“.",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        if (current.needsPermission) {
                            OutlinedButton(onClick = { openInstallPermission(context) }) { Text("Erlauben") }
                        }
                        Button(onClick = {
                            when (UpdateChecker.install(context)) {
                                UpdateInstaller.Outcome.NeedsPermission -> openInstallPermission(context)
                                UpdateInstaller.Outcome.NoInstaller -> Snackbars.show("Kein System-Installer gefunden.")
                                else -> Unit
                            }
                        }) { Text("Installieren") }
                    }
                }
                is UpdateState.Failed -> {
                    Text(current.message, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error)
                    OutlinedButton(onClick = {
                        if (current.info != null) UpdateChecker.download(context) else UpdateChecker.checkNow(context)
                    }) { Text("Erneut") }
                }
            }
        }
    }
}

private fun title(state: UpdateState): String = when (state) {
    UpdateState.Idle -> "Updates"
    UpdateState.Checking -> "Suche nach Updates …"
    is UpdateState.UpToDate -> "Smart Explorer ist aktuell"
    is UpdateState.Available -> "Version ${state.info.latest} verfügbar"
    is UpdateState.Downloading -> "Version ${state.info.latest} wird geladen"
    is UpdateState.Ready -> "Version ${state.info.latest} bereit"
    is UpdateState.Failed -> if (state.info != null) "Update nicht installiert" else "Prüfung fehlgeschlagen"
}

@Composable
private fun DownloadProgress(taskId: String?) {
    val task = rememberTask(taskId)
    val fraction = task?.let { Format.fraction(it.doneBytes, it.totalBytes) }
    LoadingBar(loading = true, fraction = fraction)
    val line = when {
        task == null -> "Wird gestartet …"
        task.totalBytes > 0 -> "${Format.size(task.doneBytes)} von ${Format.size(task.totalBytes)}"
        else -> Format.size(task.doneBytes)
    }
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(line, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        OutlinedButton(onClick = { UpdateChecker.cancelDownload() }, enabled = taskId != null) { Text("Abbrechen") }
    }
}

private fun openInstallPermission(context: Context) {
    if (!Permissions.openInstallPackagesSettings(context)) {
        Snackbars.show("Einstellung nicht verfügbar – bitte in den App-Einstellungen erlauben.")
    }
}
