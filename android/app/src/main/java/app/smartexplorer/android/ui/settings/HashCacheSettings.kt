package app.smartexplorer.android.ui.settings

import androidx.compose.foundation.clickable
import androidx.compose.material3.ListItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.launch

/**
 * "Prüfsummen-Cache leeren" (Einstellungen → Speicher): `sync.clearHashCache` clears the file digests
 * of this app and of its background worker, and the snackbar names what was not cleared.
 */
@Composable
internal fun HashCacheClearSettings() {
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    ListItem(
        headlineContent = { Text("Prüfsummen-Cache leeren") },
        supportingContent = {
            Text(
                if (busy) {
                    "Wird geleert …"
                } else {
                    "Vergisst die gespeicherten Dateiprüfsummen; der nächste Abgleich liest die Dateien neu."
                },
            )
        },
        modifier = Modifier.clickable(
            enabled = !busy,
            onClick = {
                busy = true
                scope.launch {
                    try {
                        Snackbars.show(SyncApi.clearHashCache().summary)
                    } catch (e: CoreException) {
                        Snackbars.show(e.message ?: "Prüfsummen-Cache nicht geleert")
                    } finally {
                        busy = false
                    }
                }
            },
        ),
    )
}
