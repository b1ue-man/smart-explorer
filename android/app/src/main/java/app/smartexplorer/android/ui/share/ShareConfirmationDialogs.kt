package app.smartexplorer.android.ui.share

import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import app.smartexplorer.android.api.SharePolicyApi
import app.smartexplorer.android.api.ShareRightsApi
import app.smartexplorer.android.api.ShareRoom
import app.smartexplorer.android.api.ShareMember
import app.smartexplorer.android.api.ShareRequestPolicy
import app.smartexplorer.android.api.ShareSecurityApi
import app.smartexplorer.android.api.ShareWriteGrant
import app.smartexplorer.android.api.UnconfirmedPairing

@Composable
internal fun ShareConfirmAction(
    title: String,
    message: String,
    confirm: String,
    failure: String,
    vm: ShareViewModel,
    onDismiss: () -> Unit,
    enabled: Boolean = true,
    block: suspend () -> Unit,
) {
    val action = rememberShareAction()
    ShareActionDialog(title, action, onDismiss, confirm, enabled, onConfirm = {
        action.submit(vm, failure, onDismiss, block)
    }) { Text(message) }
}

@Composable
internal fun AllowGrantDialog(grant: ShareWriteGrant, vm: ShareViewModel, onDismiss: () -> Unit) {
    ShareConfirmAction(
        title = "${grant.name.ifBlank { "Gerät" }} wieder zulassen?",
        message = "Nur diese gespeicherte Identität wird wieder zugelassen. Die gespeicherten Freigabe- und Schreibrechte gelten weiterhin. " +
            "Befehle werden nicht aktiviert.\n\nGeräte-ID: ${grant.deviceId}\nFingerabdruck: ${grant.fingerprint}\nKnoten: ${grant.nodeId.ifEmpty { "Legacy (leer)" }}",
        confirm = "Wieder zulassen",
        failure = "Kontakt nicht wieder zugelassen",
        vm = vm,
        onDismiss = onDismiss,
    ) { SharePolicyApi.allowGrantAgain(grant) }
}

@Composable
internal fun WithdrawGrantDialog(grant: ShareWriteGrant, vm: ShareViewModel, onDismiss: () -> Unit) {
    ShareConfirmAction(
        title = "Zugriff von ${grant.name.ifBlank { "Gerät" }} entziehen?",
        message = "Der Zugriff dieser Schlüssel-/Knotenidentität und ihrer gespeicherten Geräte-ID-Aliasse auf deine Freigaben wird entzogen. " +
            "Auch die Befehlsfreigabe wird entzogen. Die Identität bleibt für ein ausdrückliches Wiederzulassen gespeichert.\n\n" +
            "Geräte-ID: ${grant.deviceId}\nFingerabdruck: ${grant.fingerprint}\nKnoten: ${grant.nodeId.ifEmpty { "Legacy (leer)" }}",
        confirm = "Zugriff entziehen",
        failure = "Zugriff nicht entzogen",
        vm = vm,
        onDismiss = onDismiss,
    ) { SharePolicyApi.withdrawGrant(grant) }
}

@Composable
internal fun RoomMemberDialog(room: ShareRoom, member: ShareMember, decision: String, vm: ShareViewModel, onDismiss: () -> Unit) {
    val action = rememberShareAction()
    val label = if (decision == "block") "Sperren" else if (decision == "allow") "Wieder zulassen" else "Zulassen"
    val pinned = !member.publicKey.isNullOrBlank() && member.nodeId != null && !member.fingerprint.isNullOrBlank()
    ShareActionDialog(
        title = "$label: ${member.name.ifBlank { "Gerät" }}",
        state = action,
        onDismiss = onDismiss,
        confirmLabel = label,
        enabled = pinned,
        onConfirm = { action.submit(vm, "Mitglied nicht geändert", onDismiss) { SharePolicyApi.setRoomMember(room, member, decision) } },
    ) {
        Text(if (decision == "block") "Zugriff dieser Identität auf deine Raumfreigaben wird entzogen." else "Diese Identität wird für deine Raumfreigaben zugelassen. Die aktuellen Root- und Raumrechte gelten; Befehle werden nicht aktiviert.")
        SelectionContainer { Text("Raum: ${room.name}\nGeräte-ID: ${member.deviceId}\nFingerabdruck: ${member.fingerprint.orEmpty()}\nKnoten: ${member.nodeId.orEmpty()}") }
        if (!pinned) Text("Vollständige Identität fehlt. Schließen und Status neu laden; keine Freigabe anhand des Namens.")
    }
}

/**
 * „Automatisch annehmen erlauben…“ / „Immer fragen…“. Turning automatic acceptance on chooses the
 * rights a device gets with it (the Direct code's rights, F6); „Beim Verbinden fragen“ gives none
 * and the device waits for the rights sheet. Texts name what happens.
 */
@Composable
internal fun RequestPolicyDialog(policy: ShareRequestPolicy, vm: ShareViewModel, onDismiss: () -> Unit) {
    val turningOn = policy.requests != "AutoAccept"
    val action = rememberShareAction()
    val presets = vm.rightsPresets
    val offered = OfferedRights.from(vm.access?.execProvider, vm.status?.execProvider)
    val current = vm.status?.codeRights ?: policy.codeRights
    var chosen by remember { mutableStateOf(current?.let { RightsPresets.matching(it, presets)?.key }) }
    var touched by remember { mutableStateOf(false) }
    val shown = if (touched) presets.firstOrNull { it.key == chosen }?.rights else current
    ShareActionDialog(
        title = if (turningOn) "Anfragen automatisch annehmen?" else "Jede Anfrage bestätigen?",
        state = action,
        onDismiss = onDismiss,
        confirmLabel = if (turningOn) "Automatisch annehmen" else "Immer fragen",
        onConfirm = {
            val selection = if (turningOn && touched) presetSelection(chosen, presets, offered, System.currentTimeMillis() / 1000) else null
            val clear = turningOn && touched && selection == null && current != null
            action.submit(vm, "Anfrageeinstellung nicht gespeichert", onDismiss) {
                SharePolicyApi.setPolicy(if (turningOn) "AutoAccept" else "Ask", selection)
                if (clear) ShareRightsApi.clearCodeRights()
            }
        },
    ) {
        if (turningOn) {
            Text(
                "Geräte, die sich mit deinem Direct-Code melden, werden ohne deine Bestätigung angenommen. " +
                    "Sie erhalten genau die Rechte, die du hier wählst. Entfernte Identitäten bleiben gesperrt.",
            )
            RightsPresetChips(
                presets,
                chosen,
                onSelect = {
                    chosen = it
                    touched = true
                },
                noneLabel = "Beim Verbinden fragen",
                enabled = !action.saving,
                noneSelected = if (touched) chosen == null else current == null,
            )
            Text(codeRightsText(shown))
            Text(
                "Diese Wahl gilt auch für den Direct-Code selbst (Zeile „Rechte für Geräte, die mit deinem Direct-Code verbinden“).",
                style = androidx.compose.material3.MaterialTheme.typography.bodySmall,
            )
        } else {
            Text("Neue Geräte warten auf deine ausdrückliche Zustimmung. Vorhandene bewusste Freigaben bleiben erhalten. Die Rechte des Direct-Codes bleiben gespeichert.")
        }
    }
}

@Composable
internal fun PairingDecisionDialog(pairing: UnconfirmedPairing, revoke: Boolean, vm: ShareViewModel, onDismiss: () -> Unit) {
    ShareConfirmAction(
        title = if (revoke) "Unbestätigte Kopplung widerrufen?" else "Unbestätigte Kopplung behalten?",
        message = pairing.label + if (revoke) "\nDer bereits installierte Kontakt beziehungsweise Raum wird entfernt. Der Hinweis bleibt erhalten, falls der Entzug fehlschlägt."
        else if (!pairing.revocable) "\nBereits übergebene Raumdaten können nicht zurückgeholt werden. Bei Bedarf einen neuen Raum mit neuem Code anlegen. Nur den Hinweis schließen?"
        else "\nDie Kopplung bleibt erhalten. Nur der Hinweis zur fehlenden Bestätigung wird geschlossen.",
        confirm = if (revoke) "Widerrufen" else if (pairing.revocable) "Behalten" else "Hinweis schließen",
        failure = if (revoke) "Kopplung nicht widerrufen" else "Hinweis nicht geschlossen",
        vm = vm,
        onDismiss = onDismiss,
        enabled = !revoke || pairing.revocable,
    ) { ShareSecurityApi.resolvePairing(pairing.exchangeId, revoke) }
}
