package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.Rights
import app.smartexplorer.android.api.RightsPresetInfo
import app.smartexplorer.android.api.ShareApi
import app.smartexplorer.android.api.ShareRequestInfo

// Places that choose rights in advance or at acceptance (plan §F, F4/F5/F6): accepting a request
// (with the fingerprint), creating a room, the Direct code's line.

/**
 * „Annehmen“ of a request: the fingerprint of the asking device next to this phone's own, to
 * compare before accepting (like the desktop), and the rights it gets at once. Without a choice the
 * Direct code's rights apply, or the device waits and the rights sheet opens (F1, F5).
 */
@Composable
internal fun AcceptRequestDialog(opened: ShareRequestInfo, vm: ShareViewModel, onDismiss: () -> Unit) {
    val action = rememberShareAction()
    val status = vm.status
    // The request as the latest status shows it; one that is gone cannot be accepted any more.
    val request = if (status == null) opened
    else status.incoming.firstOrNull { it.requestId == opened.requestId } ?: opened.copy(canAccept = false, stateText = "Diese Anfrage ist nicht mehr offen.")
    val presets = vm.rightsPresets
    val offered = OfferedRights.from(vm.access?.execProvider, status?.execProvider)
    var chosen by rememberSaveable { mutableStateOf<String?>(null) }
    var compared by rememberSaveable { mutableStateOf(false) }
    val fingerprint = request.fingerprint?.takeIf { it.isNotBlank() }
    val codeRights = status?.codeRights
    val name = request.name.ifBlank { "unbekanntes Gerät" }
    ShareActionDialog(
        title = "Anfrage von $name annehmen?",
        state = action,
        onDismiss = onDismiss,
        confirmLabel = "Annehmen",
        enabled = request.canAccept && (fingerprint == null || compared),
        onConfirm = {
            val selection = presetSelection(chosen, presets, offered, System.currentTimeMillis() / 1000)
            action.submit(vm, "Nicht angenommen", onDismiss) { ShareApi.decide(request.requestId, true, selection) }
        },
    ) {
        if (request.stateText.isNotBlank()) Text(request.stateText, style = MaterialTheme.typography.bodySmall)
        if (fingerprint != null) {
            Text("Fingerabdruck von $name", style = MaterialTheme.typography.labelLarge)
            SelectionContainer { Text(fingerprint, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodyMedium) }
        } else {
            Text(
                "Diese Version der Bridge nennt den Fingerabdruck der Anfrage nicht. Vergleiche auf dem anderen Gerät den Fingerabdruck dieses Telefons.",
                style = MaterialTheme.typography.bodySmall,
            )
        }
        status?.identity?.fingerprint?.takeIf { it.isNotBlank() }?.let { own ->
            Text("Fingerabdruck dieses Telefons", style = MaterialTheme.typography.labelLarge)
            SelectionContainer { Text(own, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall) }
        }
        if (fingerprint != null) {
            ShareChoice("Ich habe den Fingerabdruck mit dem anderen Gerät verglichen", compared, !action.saving) { compared = it }
        }
        Text("Rechte", style = MaterialTheme.typography.labelLarge)
        RightsPresetChips(
            presets,
            chosen,
            onSelect = { chosen = it },
            noneLabel = if (codeRights != null) "Wie Direct-Code" else "Danach festlegen",
            enabled = !action.saving,
        )
        Text(acceptRightsText(presets.firstOrNull { it.key == chosen }, codeRights), style = MaterialTheme.typography.bodyMedium)
        Text("Erst mit dieser Wahl bekommt das Gerät Rechte; bis dahin ist es nur verbunden.", style = MaterialTheme.typography.bodySmall)
    }
}

/** „Raum erstellen“ with the rights the room gives its members, chosen in the same step (F5). */
@Composable
internal fun CreateRoomDialog(vm: ShareViewModel, onDismiss: () -> Unit) {
    val action = rememberShareAction()
    val presets = vm.rightsPresets
    val offered = OfferedRights.from(vm.access?.execProvider, vm.status?.execProvider)
    var name by rememberSaveable { mutableStateOf("") }
    var chosen by rememberSaveable { mutableStateOf<String?>(null) }
    val info = presets.firstOrNull { it.key == chosen }
    ShareActionDialog(
        title = "Raum erstellen",
        state = action,
        onDismiss = onDismiss,
        confirmLabel = "Erstellen",
        enabled = name.isNotBlank(),
        onConfirm = {
            val roomName = name.trim()
            val selection = presetSelection(chosen, presets, offered, System.currentTimeMillis() / 1000)
            action.submit(vm, "Raum nicht erstellt", onDismiss) {
                val created = ShareApi.createRoom(roomName, selection)
                vm.roomCode = RoomCodeView(roomName, created.code)
            }
        },
    ) {
        OutlinedTextField(
            value = name,
            onValueChange = { name = it },
            label = { Text("Name des Raums") },
            singleLine = true,
            enabled = !action.saving,
            modifier = Modifier.padding(top = 4.dp),
        )
        Text("Rechte für Mitglieder", style = MaterialTheme.typography.labelLarge)
        RightsPresetChips(presets, chosen, onSelect = { chosen = it }, noneLabel = "Danach festlegen", enabled = !action.saving)
        Text(
            when {
                info == null -> "Der Raum gibt zunächst nichts frei. Nach dem Erstellen legst du die Rechte im Menü „Rechte für <Raum>“ fest."
                else -> "Der Raum gibt sofort: ${rightsSummary(info.rights)}. ${info.description}"
            },
            style = MaterialTheme.typography.bodyMedium,
        )
        if (info != null && (info.rights.commands != "off" || info.rights.screenView != "off")) {
            Text("Befehle und Desktop gelten für jedes Mitglied, das du zulässt.", style = MaterialTheme.typography.bodySmall)
        }
    }
}

/**
 * The rights the Direct code gives, as one line of the „Dieses Gerät“ card: the current rights in
 * words and the preset chips. „Beim Verbinden fragen“ removes them: a device that connects then
 * waits and the rights sheet opens (F1, F5). Automatic acceptance gives the same rights (F6).
 */
@Composable
internal fun CodeRightsLine(
    codeRights: Rights?,
    presets: List<RightsPresetInfo>,
    enabled: Boolean,
    onSelect: (String?) -> Unit,
    modifier: Modifier = Modifier,
) {
    androidx.compose.foundation.layout.Column(modifier, verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(4.dp)) {
        Text("Rechte für Geräte, die mit deinem Direct-Code verbinden", style = MaterialTheme.typography.labelLarge)
        Text(codeRightsText(codeRights), style = MaterialTheme.typography.bodySmall)
        RightsPresetChips(
            presets,
            codeRights?.let { RightsPresets.matching(it, presets)?.key },
            onSelect = onSelect,
            noneLabel = "Beim Verbinden fragen",
            enabled = enabled,
            noneSelected = codeRights == null,
        )
        Text(
            "Gilt für angenommene Anfragen mit dem Direct-Code und für das automatische Annehmen. Ein PIN-Angebot und ein Raum haben eine eigene Wahl.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
