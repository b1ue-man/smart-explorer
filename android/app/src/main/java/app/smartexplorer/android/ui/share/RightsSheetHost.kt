package app.smartexplorer.android.ui.share

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import app.smartexplorer.android.api.RightsTarget
import app.smartexplorer.android.api.ShareRightsApi
import app.smartexplorer.android.ui.common.Snackbars

/**
 * The rights sheet of [request], wired to the bridge: „Bestätigen“ calls `share.confirmRights`
 * with what the draft shows, „Abbrechen“ calls `share.cancelRights` (the connection stays without
 * rights) when the connection waits, and swiping the sheet away only puts the question off. The
 * call runs in the view model, so closing the sheet cannot lose it, and a failure keeps the draft
 * with a retry.
 */
@Composable
internal fun RightsSheetHost(
    request: RightsRequest,
    vm: ShareViewModel,
    onAddFolder: (scope: String) -> Unit,
    onMore: ((RightsRequest) -> Unit)?,
) {
    val status = vm.status
    val provider = vm.access?.execProvider
    val offered = OfferedRights.from(provider, status?.execProvider)
    var draft by remember(request.key, offered) { mutableStateOf(RightsDraft.fromSeed(request.seed, request.known, offered)) }
    val action = rememberShareAction()
    val name = request.name.ifBlank { if (request.target.isRoom) "Raum" else "Gerät" }
    RightsSheet(
        request = request,
        draft = draft,
        onDraft = { draft = it },
        presets = vm.rightsPresets,
        offered = offered,
        folders = folderChoices(status, request),
        accountLabel = "App-Konto" + provider?.userLabel?.takeIf { it.isNotBlank() }?.let { " ($it)" }.orEmpty(),
        accountNote = provider?.otherAccountsNote?.takeIf { it.isNotBlank() } ?: "Befehle laufen auf Android nur als App-Konto von Smart Explorer.",
        action = action,
        nowSeconds = System.currentTimeMillis() / 1000,
        onConfirm = {
            val call = confirmCall(request, draft, vm.rightsPresets, System.currentTimeMillis() / 1000)
            action.submit(vm, "Rechte nicht gesetzt", onSaved = {
                vm.finishRights(request.key)
                Snackbars.show("Rechte für $name gesetzt")
            }) { ShareRightsApi.confirm(call.target, call.selection) }
        },
        onCancel = {
            when (val call = cancelCall(request)) {
                is RightsCall.Cancel -> action.submit(vm, "Abbruch nicht gespeichert", onSaved = {
                    vm.finishRights(request.key)
                    Snackbars.show("$name bleibt verbunden, ohne Rechte")
                }) { ShareRightsApi.cancel(call.target) }
                else -> vm.finishRights(request.key)
            }
        },
        onDismiss = { vm.dismissRights(request) },
        onAddFolder = { onAddFolder(request.folderScope) },
        onMore = onMore?.takeIf { !request.target.isRoom }?.let { more -> { more(request) } },
    )
}

/** Opens the rights sheet for [target] („Rechte einstellen“ / „Rechte festlegen“) from a menu or a page. */
internal fun ShareViewModel.openRightsFor(target: RightsTarget, name: String) {
    openRights(requestOfExisting(target, name, status, access))
}
