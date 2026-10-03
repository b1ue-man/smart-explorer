package app.smartexplorer.android.ui.share

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.SHARE_DIRECT
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.api.ShareSecurityApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.more.TextActions
import kotlinx.coroutines.launch

/** Discoverable durations offered in minutes; 5 is preselected (spec F18). */
private val OFFER_MINUTES = listOf(2, 5, 10, 15, 30)

/**
 * One-field dialog (device name, room name, access message). With [optional] an empty value is
 * confirmed as `""`; otherwise [confirmLabel] stays disabled until something is entered.
 */
@Composable
internal fun TextInputDialog(
    title: String,
    label: String,
    initial: String,
    confirmLabel: String,
    onConfirm: (String) -> Unit,
    onDismiss: () -> Unit,
    optional: Boolean = false,
    hint: String? = null,
    action: ShareActionState? = null,
) {
    var value by rememberSaveable { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = { if (action?.saving != true) onDismiss() },
        title = { Text(title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                hint?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
                OutlinedTextField(
                    value = value,
                    onValueChange = { value = it },
                    label = { Text(label) },
                    singleLine = true,
                    enabled = action?.saving != true,
                    modifier = Modifier.fillMaxWidth(),
                )
                action?.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(value.trim()) }, enabled = action?.saving != true && (optional || value.isNotBlank())) {
                Text(if (action?.saving == true) "Speichere …" else if (action?.error != null) "Erneut versuchen" else confirmLabel)
            }
        },
        dismissButton = { TextButton(onClick = onDismiss, enabled = action?.saving != true) { Text("Abbrechen") } },
    )
}

/** Code plus local name ("Raum beitreten", "Direct-Code hinzufügen"). */
@Composable
internal fun CodeDialog(
    title: String,
    codeLabel: String,
    nameDefault: String,
    confirmLabel: String,
    onConfirm: (code: String, name: String, shareBack: Boolean) -> Unit,
    onDismiss: () -> Unit,
    showShareBack: Boolean = false,
    action: ShareActionState? = null,
) {
    var code by rememberSaveable { mutableStateOf("") }
    var name by rememberSaveable { mutableStateOf(nameDefault) }
    var shareBack by rememberSaveable { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = { if (action?.saving != true) onDismiss() },
        title = { Text(title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    value = code,
                    onValueChange = { code = it },
                    label = { Text(codeLabel) },
                    singleLine = true,
                    enabled = action?.saving != true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    label = { Text("Name auf diesem Telefon") },
                    singleLine = true,
                    enabled = action?.saving != true,
                    modifier = Modifier.fillMaxWidth(),
                )
                if (showShareBack) {
                    ShareChoice("Auch meine Freigaben für dieses Gerät öffnen", shareBack, action?.saving != true) { shareBack = it }
                    Text("Ohne diese Wahl entsteht kein neuer Zugriff auf deine Freigaben. Vorhandene bewusste Rechte bleiben erhalten.")
                } else {
                    Text("Der Raum startet ohne eigene Freigaben. Freigaben und Rechte wählst du danach ausdrücklich.")
                }
                action?.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onConfirm(code.trim(), name.trim().ifBlank { nameDefault }, shareBack) },
                enabled = code.isNotBlank() && action?.saving != true,
            ) { Text(if (action?.saving == true) "Speichere …" else if (action?.error != null) "Erneut versuchen" else confirmLabel) }
        },
        dismissButton = { TextButton(onClick = onDismiss, enabled = action?.saving != true) { Text("Abbrechen") } },
    )
}

/**
 * "Suchbar machen" (spec F18): for Direct devices or a room, shown name, PIN (hint for a trivial
 * one) and duration.
 */
@Composable
internal fun DiscoverableDialog(
    status: ShareStatus,
    onStart: (target: String, alias: String, pin: String, minutes: Int, allowWeakPin: Boolean) -> Unit,
    onDismiss: () -> Unit,
    action: ShareActionState,
) {
    val coroutineScope = rememberCoroutineScope()
    var target by rememberSaveable { mutableStateOf(SHARE_DIRECT) }
    var alias by rememberSaveable { mutableStateOf(status.identity.deviceName) }
    // The PIN is a secret: kept in memory only.
    var pin by remember { mutableStateOf("") }
    var allowWeakPin by remember { mutableStateOf(false) }
    var pinError by remember { mutableStateOf<String?>(null) }
    var suggesting by remember { mutableStateOf(false) }
    var minutes by rememberSaveable { mutableStateOf(5) }
    val weak = pin.isNotEmpty() && isWeakPin(pin)
    fun suggest() {
        if (suggesting) return
        suggesting = true
        coroutineScope.launch {
            try {
                pin = ShareSecurityApi.suggestPin()
                allowWeakPin = false
                pinError = null
            } catch (e: CoreException) {
                pinError = "PIN nicht vorgeschlagen: ${e.message ?: e.kind}"
            } finally {
                suggesting = false
            }
        }
    }
    LaunchedEffect(Unit) { suggest() }
    AlertDialog(
        onDismissRequest = { if (!action.saving) onDismiss() },
        title = { Text("Suchbar machen") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Für", style = MaterialTheme.typography.labelLarge)
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilterChip(selected = target == SHARE_DIRECT, enabled = !action.saving, onClick = { target = SHARE_DIRECT }, label = { Text("Direkt-Geräte") })
                    status.rooms.forEach { room ->
                        FilterChip(
                            selected = target == room.profileId,
                            enabled = !action.saving,
                            onClick = { target = room.profileId },
                            label = { Text(room.name.ifBlank { "Raum" }) },
                        )
                    }
                }
                OutlinedTextField(
                    value = alias,
                    onValueChange = { alias = it },
                    label = { Text("Angezeigter Name") },
                    singleLine = true,
                    enabled = !action.saving,
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = pin,
                    onValueChange = { pin = it; allowWeakPin = false },
                    label = { Text("PIN") },
                    singleLine = true,
                    enabled = !action.saving && !suggesting,
                    isError = weak && !allowWeakPin,
                    supportingText = {
                        Text(if (weak) "Kurze oder leicht zu erratende PIN. Neue PIN wählen oder ausdrücklich erlauben." else "Das andere Gerät gibt diese PIN unverändert ein.")
                    },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
                    modifier = Modifier.fillMaxWidth(),
                )
                TextButton(onClick = ::suggest, enabled = !suggesting && !action.saving) { Text("Neue PIN") }
                ShareChoice("Unsichere PIN erlauben", allowWeakPin, !action.saving) { allowWeakPin = it }
                pinError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                Text("Dauer", style = MaterialTheme.typography.labelLarge)
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OFFER_MINUTES.forEach { value ->
                        FilterChip(selected = minutes == value, enabled = !action.saving, onClick = { minutes = value }, label = { Text("$value min") })
                    }
                }
                Text("Endet nach der ersten erfolgreichen Kopplung oder nach fünf Fehlversuchen; höchstens 30 Minuten.")
                action.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onStart(target, alias.trim(), pin, minutes, allowWeakPin) },
                enabled = pin.isNotEmpty() && alias.isNotBlank() && (!weak || allowWeakPin) && !action.saving && !suggesting,
            ) { Text(if (action.saving) "Starte …" else if (action.error != null) "Erneut versuchen" else "Starten") }
        },
        dismissButton = { TextButton(onClick = onDismiss, enabled = !action.saving) { Text("Abbrechen") } },
    )
}

/** Invite code of a room with [Kopieren] and [Teilen]. */
@Composable
internal fun RoomCodeDialog(view: RoomCodeView, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val code = view.code
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Raum-Code: ${view.roomName}") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (code == null) {
                    Text("Für diesen Raum ist kein Code verfügbar.")
                } else {
                    Text("Andere Geräte treten mit diesem Code bei (Raum beitreten).", style = MaterialTheme.typography.bodyMedium)
                    SelectionContainer {
                        Text(code, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodyLarge)
                    }
                }
            }
        },
        confirmButton = {
            if (code != null) {
                Row {
                    TextButton(onClick = { TextActions.copy(context, "Raum-Code", code) }) { Text("Kopieren") }
                    TextButton(onClick = { TextActions.share(context, "Smart Explorer – Raum-Code", code) }) { Text("Teilen") }
                }
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Schließen") } },
    )
}

/** Mirrors FC2's Unicode-character and repetition rules; the native validator remains authoritative. */
internal fun isWeakPin(pin: String): Boolean {
    val chars = pin.codePoints().toArray().toList()
    if (chars.size < 6) return true
    if ((1..3).any { period -> chars.size >= period * 2 && chars.indices.all { chars[it] == chars[it % period] } }) return true
    if (!chars.all { it in '0'.code..'9'.code }) return false
    val steps = chars.zipWithNext { a, b -> b - a }.toSet()
    return steps == setOf(1) || steps == setOf(-1) || pin in setOf(
        "112233", "123321", "159753", "159357", "147258", "258369", "789456", "520520", "102030", "654456",
    )
}
