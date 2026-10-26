package app.smartexplorer.android.ui.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.ui.NavRequest

/** Same object ownership as desktop E12. No ordinal is persisted or used as a legacy route. */
enum class SettingsArea(val label: String) {
    ThisDevice("Dieses Gerät"), Devices("Geräte"), Rooms("Räume"), Connections("Verbindungen"),
    Sync("Sync"), Vaults("Tresore"), Drives("Laufwerke"), Integration("Integration"),
    Appearance("Darstellung"), Updates("Updates"), Background("Hintergrund"), Diagnostics("Diagnose und Protokolle");
    val isShare: Boolean get() = this == ThisDevice || this == Devices || this == Rooms
}

/** Both settings ancestors must expose ShareScreen before it can consume this request. */
internal fun deviceSettingsAreaFor(request: NavRequest?): SettingsArea? = when (request) {
    is NavRequest.OpenApps, is NavRequest.OpenScreen, is NavRequest.ShowScreenGrid,
    NavRequest.ShowShareRequests -> SettingsArea.Devices
    else -> null
}

@Composable
fun SettingsAreaSelector(area: SettingsArea, onSelect: (SettingsArea) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
        OutlinedButton(onClick = { expanded = true }, modifier = Modifier.fillMaxWidth().semantics { heading() }) {
            Text("Bereich: ${area.label}")
        }
        DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            SettingsArea.entries.forEach { candidate ->
                DropdownMenuItem(text = { Text(candidate.label) }, onClick = { expanded = false; onSelect(candidate) })
            }
        }
    }
}
