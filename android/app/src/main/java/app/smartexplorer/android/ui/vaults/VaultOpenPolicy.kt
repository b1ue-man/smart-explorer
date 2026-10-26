package app.smartexplorer.android.ui.vaults

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.VaultApi
import app.smartexplorer.android.api.VaultOpeningPolicy
import app.smartexplorer.android.api.VaultRootBinding
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.system.documents.SeDocumentsProvider
import app.smartexplorer.android.ui.common.ConfirmDialog
import app.smartexplorer.android.ui.more.SectionHeader
import kotlinx.coroutines.launch

/** The same identity decision is reachable from properties and provider settings. */
@Composable
fun VaultOpenPolicy(siteId: String, onChanged: () -> Unit = {}) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var value by remember(siteId) { mutableStateOf<VaultOpeningPolicy?>(null) }
    var busy by remember(siteId) { mutableStateOf(false) }
    var error by remember(siteId) { mutableStateOf<String?>(null) }
    var grant by remember(siteId) { mutableStateOf<VaultRootBinding?>(null) }
    var revoke by remember(siteId) { mutableStateOf(false) }
    var release by remember(siteId) { mutableStateOf(false) }
    fun change(action: suspend () -> VaultOpeningPolicy) {
        busy = true
        error = null
        scope.launch {
            try {
                val next = action()
                value = next
                SeDocumentsProvider.setEnabled(context, next.providerEnabled)
                onChanged()
            } catch (e: CoreException) {
                error = e.message ?: e.kind
                // A revoke can persist before a later lock/reporting failure.
                value = try { VaultApi.openPolicy(siteId) } catch (_: CoreException) { null }
            } finally { busy = false }
        }
    }
    LaunchedEffect(siteId) {
        busy = true
        try { value = VaultApi.openPolicy(siteId) }
        catch (e: CoreException) { error = e.message ?: e.kind }
        finally { busy = false }
    }
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
        SectionHeader(VaultTexts.PROVIDER_POLICY)
        Text(VaultTexts.PROVIDER_BOUNDARY, style = MaterialTheme.typography.bodySmall)
        value?.let { policy ->
            val current = policy.current
            if (current != null) {
                Text(if (current.enforced) VaultTexts.ROOT_ENFORCED else VaultTexts.ROOT_UNPROVEN)
                Text(bindingText(current.binding), style = MaterialTheme.typography.bodySmall)
                Text(if (current.trusted) "Ausdrückliches Vertrauen gilt für diese Speicheridentität." else "Kein ausdrückliches Vertrauen für diese Speicheridentität.")
                if (policy.storedRootTrust && !current.trusted) Text("Gespeichertes Vertrauen gilt für eine abweichende Identität und wird nicht verwendet.")
                if (!current.enforced && !current.trusted) {
                    OutlinedButton(enabled = !busy, onClick = { grant = current.binding }) { Text(VaultTexts.TRUST_ROOT) }
                }
                if (current.read_only) Text("Der Speicher ist nur lesend nutzbar.")
            }
            if (current == null || !current.trusted) policy.storedRootBinding?.let {
                Text("Gespeichertes Vertrauen für:\n" + bindingText(it), style = MaterialTheme.typography.bodySmall)
            }
            policy.resolutionError?.let { Text("Aktuelle Speicheridentität nicht bestätigt: $it", color = MaterialTheme.colorScheme.error) }
            if (policy.storedRootTrust) TextButton(enabled = !busy, onClick = { revoke = true }) { Text(VaultTexts.REVOKE_ROOT) }
            Text(if (policy.providerReleased) "Dieser Tresor ist für den Dokumentanbieter ausgewählt." else "Dieser Tresor ist nicht für den Dokumentanbieter ausgewählt.")
            Text(if (policy.providerEnabled) "Dokumentanbieter eingeschaltet." else "Dokumentanbieter ausgeschaltet.")
            Text(if (policy.unlocked) "Tresor entsperrt." else "Tresor gesperrt; vor der Freigabe entsperren.")
            if (policy.providerReleased) {
                TextButton(enabled = !busy, onClick = { change { VaultApi.providerRelease(siteId, false) } }) { Text("Freigabe für Apps widerrufen") }
            } else {
                OutlinedButton(enabled = !busy && policy.unlocked && current != null && (current.enforced || current.trusted), onClick = { release = true }) {
                    Text("Entsperrten Tresor für Apps freigeben…")
                }
            }
        }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        if (busy) LinearProgressIndicator()
        TextButton(enabled = !busy, onClick = { change { VaultApi.openPolicy(siteId) } }) { Text("Speicheridentität erneut abfragen") }
    }
    grant?.let { binding ->
        ConfirmDialog(title = VaultTexts.TRUST_ROOT, message = bindingText(binding) + "\n\n" + VaultTexts.TRUST_BOUNDARY,
            confirmLabel = "Diesem Speicherort vertrauen", onConfirm = {
                grant = null
                change { VaultApi.confirmRootTrust(siteId, binding) }
            }, onDismiss = { grant = null })
    }
    if (revoke) ConfirmDialog(title = VaultTexts.REVOKE_ROOT,
        message = "Der Tresor wird gesperrt; offene Dokumentanbieterzugriffe werden entzogen. Ungespeicherte Änderungen werden verschlüsselt aufbewahrt.",
        confirmLabel = "Widerrufen und sperren", destructive = true, onConfirm = {
            revoke = false
            change { VaultApi.revokeRootTrust(siteId) }
        }, onDismiss = { revoke = false })
    if (release) ConfirmDialog(title = "Tresor für Apps freigeben?",
        message = "Der Dokumentanbieter wird eingeschaltet. Dieser entsperrte Tresor wird in der Systemauswahl sichtbar. Bereits ausgewählte Geräte, Verbindungen und Tresore werden bei eingeschaltetem Anbieter ebenfalls sichtbar. " + VaultTexts.PROVIDER_BOUNDARY,
        confirmLabel = "Freigeben", onConfirm = {
            release = false
            change { VaultApi.providerRelease(siteId, true) }
        }, onDismiss = { release = false })
}

private fun bindingText(binding: VaultRootBinding): String =
    "Tresor: ${binding.vault}\nSpeicherort: ${binding.location}\nBackend: ${binding.backend}\nKonto/Peer: ${binding.account}\nNamensraum: ${binding.namespace}\nRoot: ${binding.root}\nRootobjekt: ${binding.`object` ?: "keine Objektkennung verfügbar"}"
