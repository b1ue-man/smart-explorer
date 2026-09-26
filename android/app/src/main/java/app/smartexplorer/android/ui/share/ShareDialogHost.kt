package app.smartexplorer.android.ui.share

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import app.smartexplorer.android.api.ShareApi
import app.smartexplorer.android.ui.common.ConfirmDialog
import app.smartexplorer.android.ui.connections.LocalFilePickerDialog
import app.smartexplorer.android.ui.picker.LocationPickerDialog

/** Dialogs of the "Teilen" tab (spec B "Vollständige Elementliste": Suchbar machen, PIN, Raum-Code, Befehl). */
internal sealed interface ShareDialog {
    data class Rename(val current: String) : ShareDialog

    data object Discoverable : ShareDialog

    data object Connect : ShareDialog

    data object CreateRoom : ShareDialog

    data object JoinRoom : ShareDialog

    data object AddDirect : ShareDialog

    data class RequestAccess(val contactId: String, val name: String) : ShareDialog

    data class RemoveDevice(val contactId: String, val name: String) : ShareDialog

    data class LeaveRoom(val profileId: String, val name: String) : ShareDialog

    data class RemoveRoom(val profileId: String, val name: String) : ShareDialog

    data class Exec(val target: PeerTarget) : ShareDialog

    /** "Datei senden", step 1: local files. */
    data class SendPick(val target: PeerTarget) : ShareDialog

    /** "Datei senden", step 2: folder on the device. */
    data class SendTarget(val target: PeerTarget, val sources: List<String>) : ShareDialog

    /** [scope] `direct` or a room profile id. */
    data class AddExport(val scope: String) : ShareDialog
}

/**
 * Shows [dialog]. Every confirmation closes the dialog first and runs the action in the view
 * model, so it survives the dialog; [onReplace] moves to the next step of a two-step dialog.
 */
@Composable
internal fun ShareDialogHost(
    dialog: ShareDialog,
    vm: ShareViewModel,
    onDismiss: () -> Unit,
    onReplace: (ShareDialog) -> Unit,
    onShowTransfers: () -> Unit,
) {
    when (dialog) {
        is ShareDialog.Rename -> TextInputDialog(
            title = "Gerätename",
            label = "Name",
            initial = dialog.current,
            confirmLabel = "Speichern",
            onConfirm = { name ->
                onDismiss()
                vm.act("Name nicht geändert") { ShareApi.setName(name) }
            },
            onDismiss = onDismiss,
        )
        ShareDialog.Discoverable -> {
            val status = vm.status
            if (status == null) {
                // Not loaded (yet): nothing to offer; close outside of composition.
                LaunchedEffect(Unit) { onDismiss() }
            } else {
                DiscoverableDialog(
                    status,
                    onStart = { target, alias, pin, minutes ->
                        onDismiss()
                        vm.act("Nicht suchbar") { ShareApi.discoverable(target, alias, pin, minutes) }
                    },
                    onDismiss = onDismiss,
                )
            }
        }
        ShareDialog.Connect -> ConnectDeviceDialog(vm, onDismiss)
        ShareDialog.CreateRoom -> TextInputDialog(
            title = "Raum erstellen",
            label = "Name des Raums",
            initial = "",
            confirmLabel = "Erstellen",
            onConfirm = { name ->
                onDismiss()
                vm.createRoom(name)
            },
            onDismiss = onDismiss,
        )
        ShareDialog.JoinRoom -> CodeDialog(
            title = "Raum beitreten",
            codeLabel = "Raum-Code",
            nameDefault = "Raum",
            confirmLabel = "Beitreten",
            onConfirm = { code, name ->
                onDismiss()
                vm.act("Nicht beigetreten", "Raum beigetreten") { ShareApi.joinRoom(code, name) }
            },
            onDismiss = onDismiss,
        )
        ShareDialog.AddDirect -> CodeDialog(
            title = "Direct-Code hinzufügen",
            codeLabel = "Direct-Code des anderen Geräts",
            nameDefault = "Gerät",
            confirmLabel = "Hinzufügen",
            onConfirm = { code, name ->
                onDismiss()
                vm.act("Nicht hinzugefügt", "Gerät hinzugefügt") { ShareApi.addDirect(code, name) }
            },
            onDismiss = onDismiss,
        )
        is ShareDialog.RequestAccess -> TextInputDialog(
            title = "Zugriff anfragen",
            label = "Nachricht (optional)",
            initial = "",
            confirmLabel = "Anfragen",
            optional = true,
            hint = "„${dialog.name}“ bekommt eine Anfrage und kann sie annehmen oder ablehnen.",
            onConfirm = { message ->
                onDismiss()
                vm.act("Nicht angefragt", "Anfrage gesendet") { ShareApi.requestAccess(dialog.contactId, message.ifBlank { null }) }
            },
            onDismiss = onDismiss,
        )
        is ShareDialog.RemoveDevice -> ConfirmDialog(
            title = "Gerät entfernen?",
            message = "„${dialog.name}“ wird entfernt und kann sich erst nach „Wieder zulassen“ neu koppeln. " +
                "Favoriten und Tabs des Geräts werden entfernt; Sync-Jobs werden nur gemeldet.",
            confirmLabel = "Entfernen",
            destructive = true,
            onConfirm = {
                onDismiss()
                vm.removeDevice(dialog.contactId, dialog.name)
            },
            onDismiss = onDismiss,
        )
        is ShareDialog.LeaveRoom -> ConfirmDialog(
            title = "Raum verlassen?",
            message = "Dieses Telefon tritt „${dialog.name}“ nicht mehr automatisch bei. Der Raum bleibt gespeichert.",
            confirmLabel = "Verlassen",
            onConfirm = {
                onDismiss()
                vm.act("Nicht verlassen") { ShareApi.leaveRoom(dialog.profileId) }
            },
            onDismiss = onDismiss,
        )
        is ShareDialog.RemoveRoom -> ConfirmDialog(
            title = "Raum entfernen?",
            message = "„${dialog.name}“ und sein Code werden von diesem Telefon gelöscht. Favoriten und Tabs des Raums " +
                "werden entfernt; Sync-Jobs werden nur gemeldet.",
            confirmLabel = "Entfernen",
            destructive = true,
            onConfirm = {
                onDismiss()
                vm.removeRoom(dialog.profileId, dialog.name)
            },
            onDismiss = onDismiss,
        )
        is ShareDialog.Exec -> ExecDialog(dialog.target.name, dialog.target.location, onDismiss, vm = vm)
        is ShareDialog.SendPick -> LocalFilePickerDialog(
            title = "Dateien für ${dialog.target.name}",
            multiple = true,
            onPick = { paths -> onReplace(ShareDialog.SendTarget(dialog.target, paths)) },
            onDismiss = onDismiss,
        )
        is ShareDialog.SendTarget -> LocationPickerDialog(
            title = "Ziel auf ${dialog.target.name}",
            initialLocation = dialog.target.location,
            confirmLabel = "Hierhin senden",
            onPick = { targetDir ->
                onDismiss()
                vm.sendFiles(dialog.sources, targetDir, onShowTransfers)
            },
            onDismiss = onDismiss,
        )
        is ShareDialog.AddExport -> LocationPickerDialog(
            title = "Ordner freigeben",
            initialLocation = null,
            confirmLabel = "Freigeben",
            onPick = { path ->
                onDismiss()
                vm.act("Nicht freigegeben", "Ordner freigegeben") { ShareApi.addExport(dialog.scope, path, label = null) }
            },
            onDismiss = onDismiss,
        )
    }
}
