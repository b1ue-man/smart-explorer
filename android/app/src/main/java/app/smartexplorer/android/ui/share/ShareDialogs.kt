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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.SHARE_DIRECT
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.ui.more.TextActions

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
) {
    var value by rememberSaveable { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                hint?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
                OutlinedTextField(
                    value = value,
                    onValueChange = { value = it },
                    label = { Text(label) },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(value.trim()) }, enabled = optional || value.isNotBlank()) { Text(confirmLabel) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } },
    )
}

/** Code plus local name ("Raum beitreten", "Direct-Code hinzufügen"). */
@Composable
internal fun CodeDialog(
    title: String,
    codeLabel: String,
    nameDefault: String,
    confirmLabel: String,
    onConfirm: (code: String, name: String) -> Unit,
    onDismiss: () -> Unit,
) {
    var code by rememberSaveable { mutableStateOf("") }
    var name by rememberSaveable { mutableStateOf(nameDefault) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    value = code,
                    onValueChange = { code = it },
                    label = { Text(codeLabel) },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    label = { Text("Name auf diesem Telefon") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onConfirm(code.trim(), name.trim().ifBlank { nameDefault }) },
                enabled = code.isNotBlank(),
            ) { Text(confirmLabel) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } },
    )
}

/**
 * "Suchbar machen" (spec F18): for Direct devices or a room, shown name, PIN (hint for a trivial
 * one) and duration.
 */
@Composable
internal fun DiscoverableDialog(
    status: ShareStatus,
    onStart: (target: String, alias: String, pin: String, minutes: Int) -> Unit,
    onDismiss: () -> Unit,
) {
    var target by rememberSaveable { mutableStateOf(SHARE_DIRECT) }
    var alias by rememberSaveable { mutableStateOf(status.identity.deviceName) }
    // The PIN is a secret: kept in memory only.
    var pin by remember { mutableStateOf("") }
    var minutes by rememberSaveable { mutableStateOf(5) }
    val weak = pin.isNotEmpty() && isWeakPin(pin)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Suchbar machen") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Für", style = MaterialTheme.typography.labelLarge)
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilterChip(selected = target == SHARE_DIRECT, onClick = { target = SHARE_DIRECT }, label = { Text("Direkt-Geräte") })
                    status.rooms.forEach { room ->
                        FilterChip(
                            selected = target == room.profileId,
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
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = pin,
                    onValueChange = { pin = it },
                    label = { Text("PIN") },
                    singleLine = true,
                    isError = weak,
                    supportingText = {
                        Text(if (weak) "Einfache PIN – leicht zu erraten" else "Das andere Gerät gibt diese PIN ein.")
                    },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                    modifier = Modifier.fillMaxWidth(),
                )
                Text("Dauer", style = MaterialTheme.typography.labelLarge)
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OFFER_MINUTES.forEach { value ->
                        FilterChip(selected = minutes == value, onClick = { minutes = value }, label = { Text("$value min") })
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onStart(target, alias.trim(), pin, minutes) },
                enabled = pin.isNotEmpty() && alias.isNotBlank(),
            ) { Text("Starten") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } },
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

/** Short, one repeated character, or an ascending/descending digit run (e.g. 1234, 9876). */
internal fun isWeakPin(pin: String): Boolean {
    if (pin.length < 4 || pin.toSet().size == 1) return true
    if (!pin.all(Char::isDigit)) return false
    val steps = pin.zipWithNext { a, b -> b - a }.toSet()
    return steps == setOf(1) || steps == setOf(-1)
}
