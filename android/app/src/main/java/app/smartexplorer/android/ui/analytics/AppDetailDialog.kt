package app.smartexplorer.android.ui.analytics

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.AnalyzeChild
import app.smartexplorer.android.system.StorageStatsAccess
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars

/**
 * Breakdown of one app of "≈ Apps (laut Android)" (spec B6): app (APK/code with its `Android/obb`),
 * data (with its `Android/data`), the cache part of the data and [App-Info öffnen] – Android's page
 * of the app, where "Cache leeren" and "Speicher löschen" live. Apps' folders stay closed to every
 * other app, so this is the way to act on them.
 */
@Composable
internal fun AppDetailDialog(app: AnalyzeChild, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { SeIcon(R.drawable.ic_apk, contentDescription = null) },
        title = { Text(app.name) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                app.packageName?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = muted) }
                FigureRow("App (APK/Code, inkl. Android/obb)", app.appBytes)
                FigureRow("Daten (inkl. Android/data)", app.dataBytes)
                FigureRow("davon Cache", app.cacheBytes, Modifier.padding(start = 16.dp))
                HorizontalDivider()
                FigureRow("Gesamt", app.size, style = MaterialTheme.typography.titleSmall)
                Text(
                    "Werte laut Android. Cache leeren und Speicher verwalten lässt sich in der App-Info.",
                    style = MaterialTheme.typography.bodySmall,
                    color = muted,
                )
            }
        },
        confirmButton = {
            TextButton(onClick = {
                val opened = app.packageName?.let { StorageStatsAccess.openAppDetails(context, it) } == true
                if (opened) onDismiss() else Snackbars.show("App-Info nicht verfügbar.")
            }) { Text("App-Info öffnen") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Schließen") } },
    )
}

@Composable
private fun FigureRow(
    label: String,
    bytes: Long,
    modifier: Modifier = Modifier,
    style: TextStyle = MaterialTheme.typography.bodyMedium,
) {
    Row(modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(label, style = style, modifier = Modifier.weight(1f))
        Text(Format.size(bytes), style = style)
    }
}
