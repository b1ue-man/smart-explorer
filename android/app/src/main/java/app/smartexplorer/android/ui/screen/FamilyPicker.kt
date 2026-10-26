package app.smartexplorer.android.ui.screen

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.FamilyApi
import app.smartexplorer.android.api.FamilyListing
import app.smartexplorer.android.api.FamilyListEntry
import kotlinx.coroutines.CancellationException

@Composable
fun FamilyPicker(target: ScreenTarget, deviceName: String, onSelect: (ScreenTarget, FamilyListEntry) -> Unit, onBack: () -> Unit) {
    var listing by remember(target) { mutableStateOf<FamilyListing?>(null) }
    var error by remember(target) { mutableStateOf<String?>(null) }
    var busy by remember(target) { mutableStateOf(false) }
    var control by remember { mutableStateOf(target.control) }
    var refresh by remember { mutableIntStateOf(0) }
    LaunchedEffect(target, refresh) {
        busy = true; error = null; listing = null
        try {
            listing = FamilyApi.list(target)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = e.message ?: "Fensterliste konnte nicht geladen werden."
        } finally {
            busy = false
        }
    }
    AlertDialog(onDismissRequest = onBack, title = { Text("Programmfenster · $deviceName") }, text = {
        Column {
            if (busy) { CircularProgressIndicator(); Text("Fensterfreigabe und Liste werden angefragt …") }
            error?.let { Text(it) }
            androidx.compose.foundation.layout.Row {
                Checkbox(control, onCheckedChange = { control = it }); Text("Mit Steuerrecht öffnen")
            }
            listing?.let { list ->
                if (list.entries.isEmpty()) Text("Keine bestätigten Fensterfamilien verfügbar.")
                LazyColumn(Modifier.heightIn(max = 360.dp)) {
                    items(list.entries, key = { it.source }) { entry ->
                        Column(Modifier.fillMaxWidth().clickable {
                            FamilyApi.validate(entry)
                            onSelect(target.copy(source = entry.source, control = control), entry)
                        }.padding(vertical = 12.dp)) {
                            Text(entry.window.app); Text(entry.window.title)
                        }
                    }
                }
                list.limitations.forEach { Text(it) }
            }
        }
    }, confirmButton = { TextButton(enabled = !busy, onClick = { refresh++ }) { Text("Aktualisieren") } },
        dismissButton = { TextButton(onClick = onBack) { Text("Zurück") } })
}
