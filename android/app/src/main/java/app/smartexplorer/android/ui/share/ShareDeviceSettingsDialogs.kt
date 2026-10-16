package app.smartexplorer.android.ui.share

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import app.smartexplorer.android.api.ShareDevice
import app.smartexplorer.android.api.ShareSettingsApi

/**
 * „Verbindung…“ of a Direct device (desktop device page „Verbindung“, `se share device set`):
 * connect automatically, open its files once reachable, and [Vertrauen zurücksetzen…].
 */
@Composable
internal fun DeviceConnectionDialog(device: ShareDevice, vm: ShareViewModel, onDismiss: () -> Unit, onResetTrust: () -> Unit) {
    val action = rememberShareAction()
    var autoConnect by rememberSaveable(device.contactId) { mutableStateOf(device.autoConnect) }
    var autoOpen by rememberSaveable(device.contactId) { mutableStateOf(device.autoOpen) }
    val changed = autoConnect != device.autoConnect || autoOpen != device.autoOpen
    ShareActionDialog(
        title = "Verbindung: ${device.name.ifBlank { "Gerät" }}",
        state = action,
        onDismiss = onDismiss,
        enabled = changed,
        onConfirm = {
            action.submit(vm, "Verbindung nicht geändert", onDismiss) {
                ShareSettingsApi.setContactOptions(
                    device.contactId,
                    autoConnect.takeIf { it != device.autoConnect },
                    autoOpen.takeIf { it != device.autoOpen },
                )
            }
        },
    ) {
        ShareChoice("Automatisch verbinden", autoConnect) { autoConnect = it }
        ShareChoice("Dateien automatisch öffnen, sobald erreichbar", autoOpen) { autoOpen = it }
        Text(
            "Vertrauen zurücksetzen vergisst die gespeicherte Identität dieses Geräts und entzieht seinen Zugriff; " +
                "danach muss es neu bestätigt werden.",
            style = MaterialTheme.typography.bodySmall,
        )
        TextButton(onClick = onResetTrust) { Text("Vertrauen zurücksetzen…") }
    }
}
