package app.smartexplorer.android.ui.screen

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.DeviceFeed
import app.smartexplorer.android.api.FamilyApi
import app.smartexplorer.android.api.FamilyListing
import app.smartexplorer.android.api.FamilyListEntry
import app.smartexplorer.android.api.TcpApi
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay

/**
 * The open windows of a device (design 5.2, user rule r5): they come from the background
 * service's device feed without a request; a window opens with control when the device lets
 * this phone steer, otherwise viewing. A device without a feed (older version) is asked once.
 */
@Composable
fun FamilyPicker(target: ScreenTarget, deviceName: String, onSelect: (ScreenTarget, FamilyListEntry) -> Unit, onBack: () -> Unit) {
    var feed by remember(target) { mutableStateOf<DeviceFeed?>(null) }
    var asked by remember(target) { mutableStateOf<FamilyListing?>(null) }
    var error by remember(target) { mutableStateOf<String?>(null) }
    var busy by remember(target) { mutableStateOf(true) }
    LaunchedEffect(target) {
        try {
            val key = TcpApi.devices().firstOrNull {
                it.relation == target.relation && it.relationId == target.relationId && it.deviceId == target.deviceId
            }?.publicKey
            var fetched = false
            // Reads only this phone's background state; the feed itself is event-driven.
            while (true) {
                val current = if (key != null && target.deviceId != null) FamilyApi.feed(target.deviceId, key) else null
                feed = current
                if (current?.known != true && !fetched) {
                    fetched = true
                    try {
                        asked = FamilyApi.list(target)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        error = e.message ?: "Fensterliste konnte nicht geladen werden."
                    }
                }
                busy = false
                delay(1000)
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = e.message ?: "Fensterliste konnte nicht geladen werden."
        } finally {
            busy = false
        }
    }
    val control = feed?.control == true
    val listing = feed?.families ?: asked
    AlertDialog(onDismissRequest = onBack, title = { Text("Programmfenster · $deviceName") }, text = {
        Column {
            if (busy) { CircularProgressIndicator(); Text("Fensterliste wird gelesen …") }
            error?.let { Text(it) }
            feed?.unavailable?.let { Text(it) }
            feed?.familiesUnavailable?.let { Text(it) }
            listing?.let { list ->
                if (list.entries.isEmpty()) Text("Auf diesem Gerät ist gerade kein Fenster offen.")
                LazyColumn(Modifier.heightIn(max = 360.dp)) {
                    items(list.entries, key = { it.source }) { entry ->
                        Column(Modifier.fillMaxWidth().clickable {
                            FamilyApi.validate(entry)
                            onSelect(target.copy(source = entry.source, control = control), entry)
                        }.padding(vertical = 12.dp)) {
                            Text(entry.window.app); Text(entry.window.title)
                            Text(if (control) "Tippen: steuern" else "Tippen: ansehen")
                        }
                    }
                }
                list.limitations.forEach { Text(it) }
            }
        }
    }, confirmButton = { TextButton(onClick = onBack) { Text("Zurück") } })
}
