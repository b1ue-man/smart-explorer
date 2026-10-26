package app.smartexplorer.android.ui.screen

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.FamilyListEntry
import app.smartexplorer.android.screen.ScreenIconTarget

/** Navigation owner. The injected renderer is the existing Bitmap/Surface
 * ScreenViewScreen in family mode; it must call FamilyWindowMenu in its Dialog. */
@Composable
fun FamilyScreen(target: ScreenTarget, entry: FamilyListEntry, deviceName: String, onBack: () -> Unit,
    render: @Composable (FamilySession, () -> Unit) -> Unit) {
    val backend = LocalScreenBackend.current
    val family = remember(target, entry.identity) { FamilySession(target, entry, deviceName, backend) }
    DisposableEffect(family) { onDispose { family.disconnect() } }
    render(family) { family.disconnect(); onBack() }
}

/** Called inside the existing renderer after its image, so local menu taps
 * cannot enter the remote image input mapper. No permanent toolbar is added. */
@Composable
fun FamilyWindowMenu(family: FamilySession, onBack: () -> Unit,
    onLocalUiOpen: (Boolean) -> Unit = {}, onKeyboard: (() -> Unit)? = null) {
    val state by family.state.collectAsState()
    var expanded by remember { mutableStateOf(false) }
    var diagnostics by remember { mutableStateOf(false) }
    var sizing by remember { mutableStateOf(false) }
    val localOpen = expanded || diagnostics || sizing
    val notifyLocal by rememberUpdatedState(onLocalUiOpen)
    LaunchedEffect(localOpen) { notifyLocal(localOpen) }
    DisposableEffect(Unit) { onDispose { notifyLocal(false) } }
    var width by remember { mutableStateOf("") }; var height by remember { mutableStateOf("") }
    val snapshot = state.family?.snapshot
    val root = snapshot?.members?.firstOrNull { it.identity == snapshot.root }
    val canControl = family.target.control && state.canControl && state.blocked == null && state.holder == "viewer"
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.align(Alignment.TopStart).padding(8.dp)) {
            TextButton(onClick = { expanded = !expanded }) {
                ScreenAppIcon(peer = state.iconPeer,
                    target = root?.icon?.app_id?.let { ScreenIconTarget.app(it) }
                        ?: ScreenIconTarget.window(family.entry.window.id),
                    reference = root?.icon.takeIf { state.iconVersion == 1 },
                    fetch = { request -> state.iconFetch?.invoke(request) })
                Text("Menü", modifier = Modifier.padding(4.dp))
            }
            DropdownMenu(expanded, onDismissRequest = { expanded = false }) {
                Text(family.title, Modifier.padding(12.dp))
                Text("Verbindung: ${state.path ?: state.state}", Modifier.padding(horizontal = 12.dp))
                state.family?.notice?.let { Text(it, Modifier.padding(12.dp)) }
                state.errorDetail?.let { Text(it, Modifier.padding(12.dp)) }
                if (!family.target.control) Text("Ansicht ohne Steuerrecht; zum Steuern das Fenster neu auswählen.", Modifier.padding(12.dp))
                state.blocked?.let { Text("Steuerung gesperrt: $it", Modifier.padding(12.dp)) }
                DropdownMenuItem(text = { Text(if (state.holder == "viewer") "Steuerung abgeben" else "Steuerung anfordern") },
                    enabled = family.target.control && state.canControl,
                    onClick = { family.control(state.holder != "viewer"); expanded = false })
                DropdownMenuItem(text = { Text("Fensterfokus bestätigen") }, enabled = canControl && root?.operations?.focus == true,
                    onClick = { root?.let { family.focus(it.identity) }; expanded = false })
                onKeyboard?.let { keyboard -> DropdownMenuItem(text = { Text("Tastatur anzeigen") },
                    enabled = canControl, onClick = { expanded = false; keyboard() }) }
                DropdownMenuItem(text = { Text("Programmgröße ändern") }, enabled = canControl && root?.operations?.resize == true,
                    onClick = { width = root?.geometry?.width?.toString().orEmpty(); height = root?.geometry?.height?.toString().orEmpty(); sizing = true; expanded = false })
                DropdownMenuItem(text = { Text("Programm schließen") }, enabled = canControl && root?.operations?.close == true,
                    onClick = { family.closeProgram(); expanded = false })
                DropdownMenuItem(text = { Text("Fensterzustand aktualisieren") }, onClick = { family.refresh() })
                DropdownMenuItem(text = { Text("Diagnose") }, onClick = { diagnostics = true; expanded = false })
                DropdownMenuItem(text = { Text("Verbindung trennen") }, onClick = { family.disconnect(); onBack() })
            }
        }
    }
    if (sizing) AlertDialog(onDismissRequest = { sizing = false }, title = { Text("Programmgröße in Pixeln") }, text = {
        Column { OutlinedTextField(width, { width = it }, label = { Text("Breite") }); OutlinedTextField(height, { height = it }, label = { Text("Höhe") }) }
    }, confirmButton = { TextButton(enabled = (width.toIntOrNull() ?: 0) > 0 && (height.toIntOrNull() ?: 0) > 0,
        onClick = { family.resize(width.toInt(), height.toInt()); sizing = false }) { Text("Anfordern") } },
        dismissButton = { TextButton(onClick = { sizing = false }) { Text("Abbrechen") } })
    if (diagnostics) AlertDialog(onDismissRequest = { diagnostics = false }, title = { Text("Fensterdiagnose") }, text = {
        Column { Text(family.title); Text("${state.state} · ${state.blocked ?: "keine gemeldete Sperre"}")
            snapshot?.let { Text("Revision ${it.revision} · Layout ${it.layout_revision}")
                it.members.forEach { member -> Text("${member.kind}: ${member.title} · ${member.geometry.width}×${member.geometry.height}") }
                it.limitations.forEach { limit -> Text(limit) } }
        }
    }, confirmButton = { TextButton(onClick = { diagnostics = false }) { Text("Schließen") } })
}
