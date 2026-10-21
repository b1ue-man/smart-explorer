// material3: ModalBottomSheet and FilterChip may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)

package app.smartexplorer.android.ui.vaults

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.VaultApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.picker.LocationPickerDialog
import kotlinx.coroutines.launch

/**
 * „Tresor-Sync einrichten“ (U31-12, T15): the second side is a new copy se keeps, a new or empty
 * folder, or another entered site of the same vault; the job moves only encrypted files and goes
 * through `vault.syncCreate` (fixed vault values, C5), never through the general job editor.
 */
@Composable
fun VaultSyncSheet(vm: VaultViewModel, siteId: String, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    val group = vm.groups.firstOrNull { group -> group.sites.any { it.site == siteId } }
    val others = group?.sites?.filter { it.site != siteId }.orEmpty()
    var target by rememberSaveable { mutableStateOf("replica") }
    var with by rememberSaveable { mutableStateOf<String?>(null) }
    var name by rememberSaveable { mutableStateOf("Tresor ${vm.label(siteId)}") }
    var direction by rememberSaveable { mutableStateOf("both") }
    var picking by rememberSaveable { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(bottom = 24.dp).navigationBarsPadding(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(VaultTexts.SYNC_TITLE, style = MaterialTheme.typography.titleLarge)
            OutlinedTextField(value = name, onValueChange = { name = it }, label = { Text("Name") }, singleLine = true, modifier = Modifier.fillMaxWidth())
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FilterChip(selected = target == "replica", onClick = { target = "replica"; with = null }, label = { Text(VaultTexts.SYNC_REPLICA) })
                FilterChip(selected = target == "location", onClick = { target = "location"; picking = true }, label = { Text(VaultTexts.SYNC_FOLDER) })
                others.forEach { other ->
                    FilterChip(
                        selected = target == "site" && with == other.site,
                        onClick = { target = "site"; with = other.site },
                        label = { Text(other.label.ifBlank { other.site }) },
                    )
                }
            }
            if (target == "location") Text(with ?: "–", style = MaterialTheme.typography.bodySmall)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("both" to "Beide Richtungen", "a2b" to "Nur zur Kopie", "b2a" to "Nur von der Kopie").forEach { (key, text) ->
                    FilterChip(selected = direction == key, onClick = { direction = key }, label = { Text(text) })
                }
            }
            Text(VaultTexts.SYNC_CONFIRM, style = MaterialTheme.typography.bodySmall)
            error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            Button(
                enabled = target == "replica" || with != null,
                onClick = {
                    scope.launch {
                        try {
                            VaultApi.syncCreate(siteId, target, with, name, direction)
                            Snackbars.show(VaultTexts.SYNC_CREATED)
                            vm.reload()
                            onDismiss()
                        } catch (e: CoreException) {
                            error = VaultTexts.errorText(e.kind, e.message.orEmpty())
                        }
                    }
                },
            ) { Text(VaultTexts.CREATE) }
            OutlinedButton(onClick = onDismiss) { Text(VaultTexts.CANCEL) }
        }
    }
    if (picking) {
        LocationPickerDialog(
            title = VaultTexts.SYNC_FOLDER,
            initialLocation = null,
            confirmLabel = VaultTexts.CHOOSE_PLACE,
            onPick = {
                with = it
                picking = false
            },
            onDismiss = { picking = false },
        )
    }
}
