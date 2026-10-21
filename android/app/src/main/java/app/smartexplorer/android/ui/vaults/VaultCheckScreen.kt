package app.smartexplorer.android.ui.vaults

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Card
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.VaultApi
import app.smartexplorer.android.api.VaultCompareHead
import app.smartexplorer.android.api.VaultItem
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.ConfirmDialog
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.more.SubPageScaffold
import kotlinx.coroutines.launch

/** Entries fetched per call while scrolling (U31-17: pages, no fixed upper bound of the list). */
internal const val VAULT_PAGE = 200

/** The note of the core for findings a running sync can explain (`health.rs`, `SYNC_NOTE`). */
private const val SYNC_NOTE = "kann ein Zwischenstand eines laufenden Syncs sein"

/** Every entry of [head] in [group] (all groups with `null`), page by page. */
internal suspend fun loadAll(head: VaultCompareHead, group: String?): List<VaultItem> {
    val all = mutableListOf<VaultItem>()
    var offset: Int? = 0
    while (offset != null) {
        val page = VaultApi.items(head.list, head.revision, group, offset, VAULT_PAGE)
        all += page.items
        offset = page.nextOffset?.takeIf { page.items.isNotEmpty() }
    }
    return all
}

/**
 * „Tresor prüfen“ (U31-7, T21): starts at once; the findings of the group `structure` with the
 * buttons of their actions. An action asks first; one that removes names the backup the core
 * makes before (`vault-removed`). Cleartext names: `FLAG_SECURE` (S6).
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun VaultCheckScreen(vm: VaultViewModel, siteId: String, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    var head by remember { mutableStateOf<VaultCompareHead?>(null) }
    var items by remember { mutableStateOf<List<VaultItem>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var asking by remember { mutableStateOf<Pair<VaultItem, String>?>(null) }
    var done by remember { mutableStateOf<String?>(null) }
    var round by remember { mutableStateOf(0) }
    SecureWindow()
    LaunchedEffect(siteId, round) {
        items = null
        try {
            val checked = VaultApi.check(siteId)
            head = checked
            items = loadAll(checked, null)
        } catch (e: CoreException) {
            error = VaultTexts.errorText(e.kind, e.message.orEmpty(), vm.label(siteId))
        }
    }
    SubPageScaffold(title = VaultTexts.CHECK, onBack = onBack) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            error?.let { ErrorCard(it, Modifier.padding(16.dp)) }
            done?.let { Text(it, modifier = Modifier.padding(16.dp)) }
            val list = items
            when {
                list == null && error == null -> {
                    Text(VaultTexts.CHECKING, modifier = Modifier.padding(16.dp))
                    LinearProgressIndicator(Modifier.fillMaxWidth().padding(horizontal = 16.dp))
                }
                list != null && list.isEmpty() -> Text(VaultTexts.NO_PROBLEMS, modifier = Modifier.padding(16.dp).testTag("vault-check-clean"))
                list != null -> LazyColumn(Modifier.fillMaxSize()) {
                    items(list, key = { it.item }) { finding ->
                        Card(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
                            Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                Text(finding.finding ?: finding.kind, style = MaterialTheme.typography.titleSmall)
                                if (finding.path.isNotBlank()) Text(finding.path, style = MaterialTheme.typography.bodySmall)
                                if (finding.note == SYNC_NOTE) Text(VaultTexts.SYNC_MAY_EXPLAIN, style = MaterialTheme.typography.bodySmall)
                                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                    finding.actions.forEach { action ->
                                        OutlinedButton(onClick = { asking = finding to action }) { Text(VaultTexts.actionTitle(action)) }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    asking?.let { (finding, action) ->
        ConfirmDialog(
            title = VaultTexts.actionTitle(action),
            message = if (VaultTexts.removes(action)) VaultTexts.BACKUP_HINT else "",
            confirmLabel = VaultTexts.actionTitle(action),
            destructive = VaultTexts.removes(action),
            onConfirm = {
                asking = null
                scope.launch {
                    try {
                        VaultApi.resolve(siteId, finding.item, action, confirmed = true)
                        done = if (VaultTexts.removes(action)) "Sicherung: vault-removed" else null
                        round++
                    } catch (e: CoreException) {
                        error = VaultTexts.errorText(e.kind, e.message.orEmpty(), vm.label(siteId))
                    }
                }
            },
            onDismiss = { asking = null },
        )
    }
}
