package app.smartexplorer.android.launcher.se

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshots.SnapshotStateList
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.launcher.LauncherViewModel
import app.smartexplorer.android.launcher.model.Kinds
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.sqrt

/**
 * The popup of a room (`FolderPopup` style, U40-9): its members as device tiles. A tap on a member opens it like a device
 * tile does; a tap outside closes the popup. [maxColumns] is the number of columns of the home grid.
 */
@Composable
fun SeRoomPopup(vm: LauncherViewModel, profileId: String, maxColumns: Int, onClose: () -> Unit) {
    val se = vm.se
    val activity = LocalContext.current
    val ui by se.ui.collectAsStateWithLifecycle()
    // A room that left the data closes its popup (the home screen watches that, like a folder that was emptied).
    val room = ui.data?.rooms?.firstOrNull { it.profileId == profileId } ?: return
    val columns = ceil(sqrt(max(1, room.members.size).toDouble())).toInt().coerceIn(2, max(2, maxColumns))
    BoxWithConstraints(
        Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.45f)).clickable(indication = null, interactionSource = null, onClick = onClose),
        contentAlignment = Alignment.Center,
    ) {
        Surface(
            Modifier.padding(24.dp).widthIn(max = 420.dp).clickable(indication = null, interactionSource = null) {},
            shape = MaterialTheme.shapes.extraLarge,
            tonalElevation = 6.dp,
        ) {
            Column(Modifier.padding(16.dp)) {
                Text(room.name, style = MaterialTheme.typography.titleLarge, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp))
                if (room.members.isEmpty()) {
                    Text("Keine Geräte im Raum.", Modifier.padding(vertical = 16.dp), style = MaterialTheme.typography.bodyLarge)
                }
                LazyVerticalGrid(
                    columns = GridCells.Fixed(columns),
                    modifier = Modifier.fillMaxWidth().heightIn(max = this@BoxWithConstraints.maxHeight * 0.7f).padding(top = 8.dp),
                ) {
                    items(room.members, key = { it.deviceId }) { member ->
                        SeMemberTile(
                            member,
                            ui,
                            Modifier.clickable {
                                se.open(activity, SeActions.tapDevice(member.status, member.location))
                                onClose()
                            },
                        )
                    }
                }
            }
        }
    }
}

/**
 * The sheet „se-Elemente“ of the edit mode (U40-10): a check box for each device, room, vault site and „Alle
 * Bildschirme“ of the last `DEVICES`. What is on a page already is checked and cannot be unchecked; „Hinzufügen“ puts the
 * newly checked ones on the first free cells of the visible page. Without any data it asks to connect once.
 */
@Composable
fun SeSheet(vm: LauncherViewModel, onClose: () -> Unit, modifier: Modifier = Modifier) {
    val se = vm.se
    val ui by se.ui.collectAsStateWithLifecycle()
    val layout by vm.repository.layout.collectAsStateWithLifecycle()
    val data = ui.data
    val present = remember(layout) { SeSeeder.present(layout) }
    val entries = remember(data) { data?.let { SeSeeder.entries(it) }.orEmpty() }
    val chosen: SnapshotStateList<String> = remember { mutableStateListOf() }
    val maxHeight = LocalConfiguration.current.screenHeightDp.dp * 0.8f
    Box(
        Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.45f)).clickable(indication = null, interactionSource = null, onClick = onClose),
        contentAlignment = Alignment.BottomCenter,
    ) {
        Surface(
            modifier.fillMaxWidth().heightIn(max = maxHeight).clickable(indication = null, interactionSource = null) {},
            shape = MaterialTheme.shapes.extraLarge,
            tonalElevation = 6.dp,
            shadowElevation = 8.dp,
        ) {
            Column(Modifier.padding(16.dp)) {
                Text("se-Elemente", style = MaterialTheme.typography.titleMedium)
                if (data == null) {
                    Text("Verbinde dich einmal mit Smart Explorer, um deine Geräte zu sehen.", Modifier.padding(vertical = 16.dp), style = MaterialTheme.typography.bodyLarge)
                    Button(onClick = { se.connect() }) { Text("Verbinden") }
                } else {
                    LazyColumn(Modifier.fillMaxWidth().padding(top = 8.dp)) {
                        items(entries, key = { it.key }) { entry ->
                            val already = entry.key in present
                            val checked = already || entry.key in chosen
                            Row(
                                Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(enabled = !already, role = Role.Checkbox) {
                                    if (entry.key in chosen) chosen.remove(entry.key) else chosen.add(entry.key)
                                },
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Checkbox(checked = checked, onCheckedChange = null, enabled = !already)
                                Text(entry.name.ifEmpty { kindName(entry.kind) }, Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Text(kindName(entry.kind), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        }
                    }
                }
                Row(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.End) {
                    TextButton(onClick = onClose) { Text("Schließen") }
                    if (data != null) {
                        Button(
                            enabled = chosen.isNotEmpty(),
                            onClick = {
                                se.addEntries(entries.filter { it.key in chosen })
                                chosen.clear()
                                onClose()
                            },
                        ) { Text("Hinzufügen") }
                    }
                }
            }
        }
    }
}

private fun kindName(kind: String): String = when (kind) {
    Kinds.SE_DEVICE -> "Gerät"
    Kinds.SE_ROOM -> "Raum"
    Kinds.SE_VAULT -> "Tresor"
    else -> "Ansicht"
}

/**
 * The line above the dock when the se elements cannot be trusted (C4.6, C3.3): „Smart Explorer antwortet nicht“ with „Erneut“
 * after a failed bridge, „Nicht verbunden“ with „Verbinden“ while the tiles come from the cache. Nothing is shown while the
 * bridge works or when no se element is on the visible page.
 */
@Composable
fun SeStatusLine(vm: LauncherViewModel, visibleHasSe: Boolean, modifier: Modifier = Modifier) {
    val se = vm.se
    val ui by se.ui.collectAsStateWithLifecycle()
    if (!visibleHasSe) return
    val (text, action) = when {
        ui.unreachable -> "Smart Explorer antwortet nicht" to "Erneut"
        ui.bridge is BridgeState.Idle -> "Nicht verbunden" to "Verbinden"
        else -> return
    }
    Surface(modifier, shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.inverseSurface) {
        Row(Modifier.padding(start = 16.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(text, color = MaterialTheme.colorScheme.inverseOnSurface, style = MaterialTheme.typography.bodyMedium)
            TextButton(onClick = { se.connect() }) { Text(action, color = MaterialTheme.colorScheme.inversePrimary) }
        }
    }
}
