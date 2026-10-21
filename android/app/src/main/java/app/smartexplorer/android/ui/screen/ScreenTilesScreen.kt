package app.smartexplorer.android.ui.screen

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import app.smartexplorer.android.R
import app.smartexplorer.android.api.ScreenDevice
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.core.CoreException
import kotlinx.coroutines.flow.filter

/** The update intervals of the previews in milliseconds; 1 s is the default. */
private val INTERVALS = listOf(500 to R.string.screen_grid_interval_500, 1000 to R.string.screen_grid_interval_1000, 2000 to R.string.screen_grid_interval_2000, 5000 to R.string.screen_grid_interval_5000)

/** A full view opened from a tile: the device and the preview session it takes over (`null` = a new one). */
private class FullView(val device: ScreenDevice, val session: ScreenSession?)

/**
 * The group view (SCR-M9, U28-9): a tile for every device of [roomId] (all devices with `null`) with
 * a live preview. Tapping a tile opens the full view over the grid; the previews stay connected
 * while the grid is open and end with it. The host limits the number of views itself (a tile then
 * shows the text of `session_limit`); the grid sets no limit.
 */
@Composable
fun ScreenTilesScreen(roomId: String?, onClose: () -> Unit) {
    val backend = LocalScreenBackend.current
    var devices by remember { mutableStateOf<List<ScreenDevice>?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var intervalMs by remember { mutableIntStateOf(1000) }
    var full by remember { mutableStateOf<FullView?>(null) }

    LaunchedEffect(roomId) {
        suspend fun load() {
            try {
                devices = backend.devices(roomId)
                failure = null
            } catch (e: CoreException) {
                failure = e.message
            }
        }
        load()
        // The online state of the devices follows the Share status.
        Core.events.filter { it is CoreEvent.Share }.collect { load() }
    }

    Dialog(
        onDismissRequest = onClose,
        properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false, dismissOnClickOutside = false),
    ) {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
            Column(Modifier.statusBarsPadding()) {
                Row(Modifier.padding(horizontal = 12.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(R.string.screen_grid_title), style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                    TextButton(onClick = onClose, modifier = Modifier.testTag("screen_grid_close")) { Text(stringResource(R.string.screen_view_close)) }
                }
                Row(Modifier.padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(stringResource(R.string.screen_grid_refresh), style = MaterialTheme.typography.bodyMedium)
                    INTERVALS.forEach { (ms, label) ->
                        FilterChip(selected = intervalMs == ms, onClick = { intervalMs = ms }, label = { Text(stringResource(label)) })
                    }
                }
                val list = devices
                when {
                    failure != null -> Text(failure.orEmpty(), Modifier.padding(16.dp), color = MaterialTheme.colorScheme.error)
                    list != null && list.isEmpty() -> Text(stringResource(R.string.screen_grid_empty), Modifier.padding(16.dp))
                    list != null -> LazyVerticalGrid(
                        columns = GridCells.Adaptive(minSize = 160.dp),
                        modifier = Modifier.fillMaxSize(),
                        contentPadding = androidx.compose.foundation.layout.PaddingValues(12.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        items(list, key = { "${it.relation}/${it.relationId}/${it.deviceId}" }) { device ->
                            ScreenPreviewTile(device, intervalMs, full?.session) { opened, preview -> full = FullView(opened, preview) }
                        }
                    }
                    else -> Box(Modifier.fillMaxSize())
                }
            }
        }
        full?.let { open ->
            val title = listOfNotNull(open.device.name, open.device.roomName).joinToString(" · ")
            ScreenViewScreen(open.device.toTarget(), title, onClose = { full = null }, adopted = open.session)
        }
    }
}
