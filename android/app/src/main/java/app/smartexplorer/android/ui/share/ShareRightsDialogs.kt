package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import app.smartexplorer.android.api.SHARE_READ_ONLY
import app.smartexplorer.android.api.SHARE_READ_WRITE
import app.smartexplorer.android.api.ShareDevice
import app.smartexplorer.android.api.ShareExport
import app.smartexplorer.android.api.SharePolicyApi
import app.smartexplorer.android.api.ShareRoom
import app.smartexplorer.android.api.ShareWriteGrant

internal fun accessLabel(access: String?): String = when (access) {
    SHARE_READ_ONLY -> "Nur lesen"
    SHARE_READ_WRITE -> "Lesen und Schreiben"
    else -> "Rechte unbekannt – bitte neu laden"
}

@Composable
internal fun ShareAccessPicker(access: String?, enabled: Boolean, onChange: (String) -> Unit) {
    Column {
        listOf(SHARE_READ_ONLY, SHARE_READ_WRITE).forEach { choice ->
            FilterChip(selected = access == choice, enabled = enabled, onClick = { onChange(choice) }, label = { Text(accessLabel(choice)) })
        }
    }
}

@Composable
internal fun ExportAccessDialog(scope: String, export: ShareExport, vm: ShareViewModel, onDismiss: () -> Unit, restoreHome: Boolean = false) {
    val action = rememberShareAction()
    var access by remember { mutableStateOf(if (restoreHome) SHARE_READ_WRITE else export.access) }
    var system by remember { mutableStateOf(export.allowSystemWrites) }
    var understood by remember { mutableStateOf(false) }
    val needsWarning = system == true && export.allowSystemWrites != true
    val dirty = access != export.access || system != export.allowSystemWrites
    ShareActionDialog(
        title = "Rechte für ${export.label.ifBlank { export.path }}",
        state = action,
        onDismiss = onDismiss,
        enabled = dirty && access in setOf(SHARE_READ_ONLY, SHARE_READ_WRITE) && (!needsWarning || understood),
        onConfirm = {
            val chosen = access
            val systemWrites = system.takeIf { it != export.allowSystemWrites }
            val expectedAccess = export.access.takeIf { chosen == it && systemWrites != null }
            if (chosen != null) action.submit(vm, "Freigaberechte nicht gespeichert", onDismiss) {
                SharePolicyApi.setExportAccess(scope, export.path, chosen, systemWrites, expectedAccess)
            }
        },
    ) {
        SelectionContainer { Text(export.path) }
        if (restoreHome) Text("Die einmalig nur lesbar gemachte Home-Freigabe wird nach deiner Bestätigung wieder schreibbar. Kontakt- und Raumrechte werden dabei nicht geändert.")
        ShareAccessPicker(access, !action.saving) { access = it }
        Text("Schreiben benötigt außerdem das Schreibrecht des Kontakts beziehungsweise Raums und die Rechte des Speichers.")
        ShareChoice("Schreiben in Autostart-, Anmelde- und Schlüsselorte erlauben", system == true, !action.saving) {
            system = it
            understood = false
        }
        if (needsWarning) {
            Text("Damit können andere Geräte in diesen Orten Programme starten oder Schlüssel verändern. Eigene App-Daten bleiben geschützt.")
            ShareChoice("Ich möchte diese Systemorte ausdrücklich beschreibbar machen", understood, !action.saving) { understood = it }
        }
    }
}

@Composable
internal fun ContactWriteDialog(grant: ShareWriteGrant, vm: ShareViewModel, onDismiss: () -> Unit) {
    val action = rememberShareAction()
    var write by remember { mutableStateOf(grant.write) }
    ShareActionDialog(
        title = "Schreibrecht für ${grant.name.ifBlank { "Gerät" }}",
        state = action,
        onDismiss = onDismiss,
        enabled = write != grant.write && grant.canSetWrite && (!write || grant.active),
        onConfirm = {
            val chosen = write
            action.submit(vm, "Kontaktrecht nicht gespeichert", onDismiss) { SharePolicyApi.setContactWrite(grant, chosen) }
        },
    ) {
        SelectionContainer { Text("Geräte-ID: ${grant.deviceId}\nFingerabdruck: ${grant.fingerprint}\nKnoten: ${grant.nodeId.ifEmpty { "Legacy (leer)" }}\nSchlüssel: ${grant.publicKey}") }
        ShareChoice("Schreiben auf schreibbaren Freigaben erlauben", write, !action.saving && (grant.active || write)) { write = it }
        Text(if (grant.active) "Nur diese gespeicherte Identität erhält das Recht. Nur-lesbare Freigaben bleiben geschützt." else "Die Freigabe ist inaktiv. Ein altes Schreibrecht kann entzogen werden; neue Rechte erst nach ausdrücklichem Wiederzulassen.")
        Text("Dieses Recht erlaubt keine Befehle.")
    }
}

@Composable
internal fun ShareBackDialog(device: ShareDevice, vm: ShareViewModel, onDismiss: () -> Unit) {
    val action = rememberShareAction()
    var shareBack by remember { mutableStateOf(device.shareBack) }
    ShareActionDialog(
        title = "Eigene Freigaben für ${device.name.ifBlank { "Gerät" }}",
        state = action,
        onDismiss = onDismiss,
        enabled = shareBack != device.shareBack,
        onConfirm = {
            val chosen = shareBack
            action.submit(vm, "Rückfreigabe nicht gespeichert", onDismiss) { SharePolicyApi.setShareBack(device.contactId, chosen) }
        },
    ) {
        ShareChoice("Auch meine Freigaben für dieses Gerät öffnen", shareBack, !action.saving) { shareBack = it }
        Text("Einschalten ist eine bewusste Freigabe und kann einen vorherigen Entzug aufheben. Es erlaubt keine Befehle.")
        Text("Ausschalten nimmt frühere bewusste Freigaben nicht zurück. Zum Entziehen das Gerät entfernen.")
    }
}

@Composable
internal fun RoomPolicyDialog(room: ShareRoom, vm: ShareViewModel, onDismiss: () -> Unit) {
    val action = rememberShareAction()
    var write by remember { mutableStateOf(room.policy?.membersMayWrite) }
    var confirm by remember { mutableStateOf(room.policy?.confirmNewMembers) }
    val dirty = write != room.policy?.membersMayWrite || confirm != room.policy?.confirmNewMembers
    ShareActionDialog(
        title = "Rechte im Raum ${room.name.ifBlank { "Raum" }}",
        state = action,
        onDismiss = onDismiss,
        enabled = dirty && write != null && confirm != null,
        onConfirm = {
            val chosenWrite = write.takeIf { it != room.policy?.membersMayWrite }
            val chosenConfirm = confirm.takeIf { it != room.policy?.confirmNewMembers }
            action.submit(vm, "Raumpolicy nicht gespeichert", onDismiss) { SharePolicyApi.setRoomPolicy(room, chosenWrite, chosenConfirm) }
        },
    ) {
        ShareChoice("Mitglieder dürfen auf schreibbaren Freigaben schreiben", write == true, !action.saving && write != null) { write = it }
        ShareChoice("Neue Mitglieder einzeln bestätigen", confirm == true, !action.saving && confirm != null) { confirm = it }
        Text("Wartende und gesperrte Mitglieder bleiben bis zum ausdrücklichen Zulassen inaktiv. Änderungen erlauben keine Befehle.")
        if (write == null || confirm == null) Text("Raumrechte nicht bekannt. Schließen und Status erneut laden.")
        SelectionContainer { Text("Profil: ${room.profileId}\nRaum: ${room.roomId}") }
    }
}
