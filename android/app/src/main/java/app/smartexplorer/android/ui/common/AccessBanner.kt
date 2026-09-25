package app.smartexplorer.android.ui.common

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import app.smartexplorer.android.R
import app.smartexplorer.android.system.Permissions

/** `true` while "Zugriff auf alle Dateien" is granted; re-checked whenever the UI resumes. */
@Composable
fun rememberAllFilesAccess(): Boolean {
    var granted by remember { mutableStateOf(Permissions.hasAllFilesAccess()) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { granted = Permissions.hasAllFilesAccess() }
    return granted
}

/**
 * Hint banner for the file view when all-files access is missing (spec F1); renders nothing
 * while access is granted.
 */
@Composable
fun AllFilesAccessBanner(modifier: Modifier = Modifier) {
    if (rememberAllFilesAccess()) return
    val context = LocalContext.current
    Surface(color = MaterialTheme.colorScheme.secondaryContainer, modifier = modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier.padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            SeIcon(R.drawable.ic_lock, contentDescription = null)
            Text(
                "Ohne Dateizugriff sind nur App-Ordner sichtbar.",
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.weight(1f),
            )
            TextButton(onClick = {
                if (!Permissions.openAllFilesAccessSettings(context)) {
                    Snackbars.show("Einstellung nicht verfügbar – bitte in den App-Einstellungen erlauben.")
                }
            }) { Text("Erlauben") }
        }
    }
}
