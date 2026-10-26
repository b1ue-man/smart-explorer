package app.smartexplorer.android.ui.share

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import app.smartexplorer.android.api.ExecTarget
import app.smartexplorer.android.api.ShareApi
import app.smartexplorer.android.api.ShareDevice
import app.smartexplorer.android.api.ShareExport
import app.smartexplorer.android.api.ShareMember
import app.smartexplorer.android.api.ShareRequestPolicy
import app.smartexplorer.android.api.ShareRoom
import app.smartexplorer.android.api.ShareSettingsApi
import app.smartexplorer.android.api.ShareWriteGrant
import app.smartexplorer.android.api.RemovedDevice
import app.smartexplorer.android.api.SHARE_DIRECT
import app.smartexplorer.android.api.UnconfirmedPairing
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

    data class ConfirmExport(val scope: String, val path: String) : ShareDialog
    data class ExportAccess(val scope: String, val export: ShareExport, val restoreHome: Boolean = false) : ShareDialog
    data class Connections(val scope: String) : ShareDialog
    data class ContactWrite(val grant: ShareWriteGrant) : ShareDialog
    data class AllowGrant(val grant: ShareWriteGrant) : ShareDialog
    data class WithdrawGrant(val grant: ShareWriteGrant) : ShareDialog
    data class ShareBack(val device: ShareDevice) : ShareDialog
    data class RoomPolicy(val room: ShareRoom) : ShareDialog
    data class RoomMember(val room: ShareRoom, val member: ShareMember, val action: String) : ShareDialog
    data class Readmit(val device: RemovedDevice) : ShareDialog
    data class PairingDecision(val pairing: UnconfirmedPairing, val revoke: Boolean) : ShareDialog
    data class RequestPolicy(val policy: ShareRequestPolicy) : ShareDialog
    data class DeviceSettings(val name: String, val contactId: String? = null, val deviceId: String? = null, val publicKey: String? = null) : ShareDialog
    data class AccessSource(val targetKey: String) : ShareDialog
    data class DeviceConnection(val device: ShareDevice) : ShareDialog
    data class Tcp(val name: String, val contactId: String? = null, val deviceId: String? = null, val publicKey: String? = null) : ShareDialog
    data class Rdp(val name: String, val contactId: String? = null, val deviceId: String? = null, val publicKey: String? = null) : ShareDialog
    data class Apps(val name: String, val contactId: String? = null, val deviceId: String? = null, val publicKey: String? = null, val endpoint: String? = null) : ShareDialog
    data class Families(val name: String, val contactId: String?, val deviceId: String?, val publicKey: String?) : ShareDialog
    data class FamilyView(val target: app.smartexplorer.android.ui.screen.ScreenTarget, val entry: app.smartexplorer.android.api.FamilyListEntry, val name: String) : ShareDialog
    data class AppWindow(val confirmed: app.smartexplorer.android.api.ConfirmedRemoteAppWindow) : ShareDialog
    data object LoginSetup : ShareDialog
    data class LoginView(val target: app.smartexplorer.android.api.LoginTarget, val user: String) : ShareDialog
    data class AppGrant(val name: String, val deviceId: String, val publicKey: String, val source: kotlinx.serialization.json.JsonObject? = null) : ShareDialog
    data class ResetTrust(val device: ShareDevice) : ShareDialog
    data class RenameRoom(val room: ShareRoom) : ShareDialog
    data class ResetMemberTrust(val room: ShareRoom, val member: ShareMember) : ShareDialog
    data object RenewCode : ShareDialog
    data class ExecPrograms(val target: ExecTarget) : ShareDialog
    data object ExecLog : ShareDialog
}

/**
 * Mutations close only after success, keeping drafts retryable. Actions run in the ViewModel;
 * [onReplace] moves to the next step of a picker without losing its selected locator.
 */
@Composable
internal fun ShareDialogHost(
    dialog: ShareDialog,
    vm: ShareViewModel,
    onDismiss: () -> Unit,
    onReplace: (ShareDialog) -> Unit,
    onShowTransfers: () -> Unit,
) {
    val action = rememberShareAction()
    when (dialog) {
        is ShareDialog.Tcp -> TcpDialog(dialog, onDismiss)
        is ShareDialog.Rdp -> RdpDialog(dialog, onDismiss)
        is ShareDialog.Apps -> AppsDialog(dialog, onDismiss, onOpenWindow = { onReplace(ShareDialog.AppWindow(it)) })
        is ShareDialog.Families -> FamilyTargetsDialog(dialog, onDismiss, onReplace)
        is ShareDialog.FamilyView -> app.smartexplorer.android.ui.screen.FamilyScreen(dialog.target, dialog.entry, dialog.name, onDismiss) { family, close ->
            app.smartexplorer.android.ui.screen.ScreenViewScreen(family.target, family.title, close, family = family)
        }
        is ShareDialog.AppWindow -> ConfirmedAppWindowDialog(dialog.confirmed, onDismiss, onReplace)
        ShareDialog.LoginSetup -> app.smartexplorer.android.ui.login.LoginTargetDialog(onDismiss) { target, user -> onReplace(ShareDialog.LoginView(target, user)) }
        is ShareDialog.LoginView -> app.smartexplorer.android.ui.login.LoginViewScreen(dialog.target, dialog.user, onDismiss)
        is ShareDialog.AppGrant -> AppGrantDialog(dialog, onDismiss)
        is ShareDialog.DeviceSettings -> DeviceSettingsPage(dialog, vm, onDismiss)
        is ShareDialog.AccessSource -> {
            val target = vm.status?.execTargets?.firstOrNull { it.targetKey == dialog.targetKey }
            if (target == null) {
                ShareActionDialog("Quelle nicht mehr vorhanden", action, onDismiss, enabled = false, onConfirm = {}) {
                    androidx.compose.material3.Text("Die Geräteidentität oder Beziehung wurde geändert. Schließen und neu laden.")
                }
            } else AccessDialog(target, vm.access?.accessOf(target.targetKey), vm.access?.execProvider,
                onConfirm = { change -> action.submit(vm, "Rechte nicht gespeichert", onDismiss) { app.smartexplorer.android.api.ShareAccessApi.setAccess(target.targetKey, change) } },
                onDismiss = onDismiss, action = action)
        }
        is ShareDialog.Rename -> TextInputDialog(
            title = "Gerätename",
            label = "Name",
            initial = dialog.current,
            confirmLabel = "Speichern",
            onConfirm = { name ->
                action.submit(vm, "Name nicht geändert", onDismiss) { ShareApi.setName(name) }
            },
            onDismiss = onDismiss,
            action = action,
        )
        ShareDialog.Discoverable -> {
            val status = vm.status
            if (status == null) {
                // Not loaded (yet): nothing to offer; close outside of composition.
                LaunchedEffect(Unit) { onDismiss() }
            } else {
                DiscoverableDialog(
                    status,
                    onStart = { target, alias, pin, minutes, allowWeakPin ->
                        action.submit(vm, "Nicht suchbar", onDismiss) { ShareApi.discoverable(target, alias, pin, minutes, allowWeakPin) }
                    },
                    onDismiss = onDismiss,
                    action = action,
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
                action.submit(vm, "Raum nicht erstellt", onDismiss) {
                    val created = ShareApi.createRoom(name)
                    vm.roomCode = RoomCodeView(name, created.code)
                }
            },
            onDismiss = onDismiss,
            hint = "Der Raum startet ohne eigene Freigaben und ohne Schreibrecht für Mitglieder.",
            action = action,
        )
        ShareDialog.JoinRoom -> CodeDialog(
            title = "Raum beitreten",
            codeLabel = "Raum-Code",
            nameDefault = "Raum",
            confirmLabel = "Beitreten",
            onConfirm = { code, name, _ ->
                action.submit(vm, "Nicht beigetreten", onDismiss) { ShareApi.joinRoom(code, name) }
            },
            onDismiss = onDismiss,
            action = action,
        )
        ShareDialog.AddDirect -> CodeDialog(
            title = "Direct-Code hinzufügen",
            codeLabel = "Direct-Code des anderen Geräts",
            nameDefault = "Gerät",
            confirmLabel = "Hinzufügen",
            onConfirm = { code, name, shareBack ->
                action.submit(vm, "Nicht hinzugefügt", onDismiss) { ShareApi.addDirect(code, name, shareBack) }
            },
            onDismiss = onDismiss,
            showShareBack = true,
            action = action,
        )
        is ShareDialog.RequestAccess -> TextInputDialog(
            title = "Zugriff anfragen",
            label = "Nachricht (optional)",
            initial = "",
            confirmLabel = "Anfragen",
            optional = true,
            hint = "„${dialog.name}“ bekommt eine Anfrage und kann sie annehmen oder ablehnen.",
            onConfirm = { message ->
                action.submit(vm, "Nicht angefragt", onDismiss) { ShareApi.requestAccess(dialog.contactId, message.ifBlank { null }) }
            },
            onDismiss = onDismiss,
            action = action,
        )
        is ShareDialog.RemoveDevice -> ShareConfirmAction(
            title = "Gerät entfernen?",
            message = "„${dialog.name}“ wird entfernt und kann sich erst nach „Wieder zulassen“ neu koppeln. " +
                "Favoriten und Tabs des Geräts werden entfernt; Sync-Jobs werden nur gemeldet.",
            confirm = "Entfernen",
            failure = "Gerät nicht entfernt",
            vm = vm,
            onDismiss = onDismiss,
        ) { vm.report(dialog.name, ShareApi.removeDevice(dialog.contactId)) }
        is ShareDialog.LeaveRoom -> ShareConfirmAction(
            title = "Raum verlassen?",
            message = "Dieses Telefon tritt „${dialog.name}“ nicht mehr automatisch bei. Der Raum bleibt gespeichert.",
            confirm = "Verlassen",
            failure = "Raum nicht verlassen",
            vm = vm,
            onDismiss = onDismiss,
        ) { ShareApi.leaveRoom(dialog.profileId) }
        is ShareDialog.RemoveRoom -> ShareConfirmAction(
            title = "Raum entfernen?",
            message = "„${dialog.name}“ und sein Code werden von diesem Telefon gelöscht. Favoriten und Tabs des Raums " +
                "werden entfernt; Sync-Jobs werden nur gemeldet.",
            confirm = "Entfernen",
            failure = "Raum nicht entfernt",
            vm = vm,
            onDismiss = onDismiss,
        ) { vm.report(dialog.name, ShareApi.removeRoom(dialog.profileId)) }
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
            confirmLabel = "Ordner wählen",
            onPick = { path -> onReplace(ShareDialog.ConfirmExport(dialog.scope, path)) },
            onDismiss = onDismiss,
        )
        // New Direct exports are read-write (own accepted devices), room exports read-only.
        is ShareDialog.ConfirmExport -> if (dialog.scope == SHARE_DIRECT) {
            ShareConfirmAction(
                title = "Ordner für deine Geräte freigeben?",
                message = "${dialog.path}\n\nDeine angenommenen Geräte dürfen hier lesen und schreiben. „Nur lesen“ wählst du anschließend unter Rechte.",
                confirm = "Freigeben",
                failure = "Ordner nicht freigegeben",
                vm = vm,
                onDismiss = onDismiss,
            ) { ShareApi.addExport(dialog.scope, dialog.path, label = null) }
        } else {
            ShareConfirmAction(
                title = "Ordner nur lesend freigeben?",
                message = "${dialog.path}\n\nDieser Ordner wird nur lesbar. Schreibrecht wählst du anschließend ausdrücklich unter Rechte.",
                confirm = "Nur lesend freigeben",
                failure = "Ordner nicht freigegeben",
                vm = vm,
                onDismiss = onDismiss,
            ) { ShareApi.addExport(dialog.scope, dialog.path, label = null) }
        }
        is ShareDialog.ExportAccess -> ExportAccessDialog(dialog.scope, dialog.export, vm, onDismiss, dialog.restoreHome)
        is ShareDialog.Connections -> ShareConnectionDialog(dialog.scope, vm, onDismiss)
        is ShareDialog.ContactWrite -> ContactWriteDialog(dialog.grant, vm, onDismiss)
        is ShareDialog.AllowGrant -> AllowGrantDialog(dialog.grant, vm, onDismiss)
        is ShareDialog.WithdrawGrant -> WithdrawGrantDialog(dialog.grant, vm, onDismiss)
        is ShareDialog.ShareBack -> ShareBackDialog(dialog.device, vm, onDismiss)
        is ShareDialog.RoomPolicy -> RoomPolicyDialog(dialog.room, vm, onDismiss)
        is ShareDialog.RoomMember -> RoomMemberDialog(dialog.room, dialog.member, dialog.action, vm, onDismiss)
        is ShareDialog.PairingDecision -> PairingDecisionDialog(dialog.pairing, dialog.revoke, vm, onDismiss)
        is ShareDialog.RequestPolicy -> RequestPolicyDialog(dialog.policy, vm, onDismiss)
        is ShareDialog.DeviceConnection -> DeviceConnectionDialog(dialog.device, vm, onDismiss, onResetTrust = {
            onReplace(ShareDialog.ResetTrust(dialog.device))
        })
        is ShareDialog.ResetTrust -> ShareConfirmAction(
            title = "Vertrauen zu ${dialog.device.name.ifBlank { "Gerät" }} zurücksetzen?",
            message = "Die gespeicherte Identität dieses Geräts wird vergessen und sein Zugriff auf deine Freigaben entzogen " +
                "(Befehle aus). Das Gerät bleibt in der Liste und muss neu bestätigt werden.",
            confirm = "Zurücksetzen",
            failure = "Vertrauen nicht zurückgesetzt",
            vm = vm,
            onDismiss = onDismiss,
        ) { ShareSettingsApi.resetContactTrust(dialog.device.contactId) }
        is ShareDialog.RenameRoom -> TextInputDialog(
            title = "Raum umbenennen",
            label = "Name des Raums",
            initial = dialog.room.name,
            confirmLabel = "Speichern",
            onConfirm = { name ->
                action.submit(vm, "Raum nicht umbenannt", onDismiss) { ShareSettingsApi.renameRoom(dialog.room.profileId, name) }
            },
            onDismiss = onDismiss,
            hint = "Der Name gilt nur auf diesem Telefon.",
            action = action,
        )
        is ShareDialog.ResetMemberTrust -> ShareConfirmAction(
            title = "Vertrauen zu ${dialog.member.name.ifBlank { "Gerät" }} zurücksetzen?",
            message = "Die zuletzt gesehene Adresse dieses Mitglieds wird vergessen und neu gesucht. Zulassung und Befehle bleiben, wie sie sind.",
            confirm = "Zurücksetzen",
            failure = "Mitglied nicht zurückgesetzt",
            vm = vm,
            onDismiss = onDismiss,
        ) { ShareSettingsApi.resetMemberTrust(dialog.room.profileId, dialog.member.deviceId) }
        ShareDialog.RenewCode -> ShareConfirmAction(
            title = "Direct-Code erneuern?",
            message = "Der bisherige Direct-Code gilt danach nicht mehr. Bestehende Direkt-Freigaben pausieren bis zur erneuten " +
                "Bestätigung, ihre Befehle werden ausgeschaltet. Der Share-Dienst startet dafür kurz neu.",
            confirm = "Erneuern",
            failure = "Direct-Code nicht erneuert",
            vm = vm,
            onDismiss = onDismiss,
        ) { ShareSettingsApi.renewCode() }
        is ShareDialog.ExecPrograms -> ExecProgramsDialog(dialog.target, vm, onDismiss)
        ShareDialog.ExecLog -> ExecAuditDialog(onDismiss)
        is ShareDialog.Readmit -> ShareConfirmAction(
            title = "${dialog.device.name.ifBlank { "Gerät" }} wieder zulassen?",
            message = "Die gespeicherte Identität darf sich wieder koppeln. Befehle werden nicht aktiviert.\n${dialog.device.deviceId}",
            confirm = "Wieder zulassen",
            failure = "Gerät nicht wieder zugelassen",
            vm = vm,
            onDismiss = onDismiss,
        ) { ShareApi.readmit(dialog.device.deviceId) }
    }
}
