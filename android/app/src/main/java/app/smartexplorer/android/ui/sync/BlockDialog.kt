package app.smartexplorer.android.ui.sync

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import app.smartexplorer.android.api.SyncJob
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

@Composable
internal fun BlockDialog(job: SyncJob, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    val block = job.state?.blocked ?: return
    val kind = (block.kind["type"] as? JsonPrimitive)?.contentOrNull
    val canConfirm = kind in setOf("mass_delete", "delete_limit", "side_empty", "replica_missing")
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("„${job.name}“ prüfen") },
        text = { Text(block.detail + if (canConfirm) "\n\nDiese Freigabe gilt einmal für genau den angezeigten Sicherheitsstopp." else "\n\nBitte Einstellungen und gespeicherten Zustand prüfen.") },
        confirmButton = {
            if (canConfirm) TextButton(onClick = onConfirm) { Text("Einmal trotzdem ausführen") }
            else TextButton(onClick = onDismiss) { Text("Schließen") }
        },
        dismissButton = { if (canConfirm) TextButton(onClick = onDismiss) { Text("Zurück") } },
    )
}
