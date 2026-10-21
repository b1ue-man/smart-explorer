package app.smartexplorer.android.ui.vaults

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Card
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.VaultApi
import app.smartexplorer.android.api.VaultCompareHead
import app.smartexplorer.android.api.VaultItem
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.ConfirmDialog
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.SubPageScaffold
import kotlinx.coroutines.launch

/** Loaded entries of one group and where the next page starts (`null` = all there). */
private class GroupPage(val items: List<VaultItem>, val next: Int?)

/**
 * Comparison of a vault sync job (U31-8, T18–T20): groups as sections in the order of the screen,
 * entries page by page while scrolling, collective actions, the detail of an entry. A locked vault
 * shows „<Bezeichnung> ist gesperrt.“ with „Entsperren“; the comparison starts again after the
 * unlock. A lock while it is open ends it (U31-9). Cleartext names: `FLAG_SECURE` (S6).
 */
@Composable
fun VaultCompareScreen(vm: VaultViewModel, jobId: String, onBack: () -> Unit, onUnlock: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    var head by remember { mutableStateOf<VaultCompareHead?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var lockedSite by remember { mutableStateOf<String?>(null) }
    var round by remember { mutableStateOf(0) }
    var detail by remember { mutableStateOf<VaultItem?>(null) }
    var askAll by remember { mutableStateOf<String?>(null) }
    val pages = remember { mutableStateMapOf<String, GroupPage>() }
    val siteOfJob = vm.sites.firstOrNull { jobId in it.syncJobs }
    SecureWindow()
    // Unlocking the vault of a locked comparison starts it again (U31-8).
    val unlockedNow = siteOfJob?.unlocked == true
    LaunchedEffect(jobId, round, unlockedNow) {
        head = null
        error = null
        lockedSite = null
        pages.clear()
        try {
            head = VaultApi.compare(jobId)
        } catch (e: CoreException) {
            if (e.kind == "vault_locked") lockedSite = siteOfJob?.site else error = VaultTexts.errorText(e.kind, e.message.orEmpty())
        }
    }
    LaunchedEffect(head) {
        val current = head ?: return@LaunchedEffect
        vm.locked.collect { event ->
            if (event.site == current.siteA || event.site == current.siteB) {
                Snackbars.show(VaultTexts.LOCKED_DURING_COMPARE)
                onBack()
            }
        }
    }
    suspend fun loadMore(group: String) {
        val current = head ?: return
        val known = pages[group]
        val offset = known?.next ?: if (known == null) 0 else return
        try {
            val page = VaultApi.items(current.list, current.revision, group, offset, VAULT_PAGE)
            pages[group] = GroupPage(known?.items.orEmpty() + page.items, page.nextOffset)
        } catch (e: CoreException) {
            error = VaultTexts.errorText(e.kind, e.message.orEmpty())
        }
    }
    val open = detail
    if (open != null && head != null) {
        VaultCompareDetail(head!!, open, onBack = { detail = null }, onDone = {
            detail = null
            round++
        })
        return
    }
    SubPageScaffold(title = VaultTexts.COMPARE, onBack = onBack) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            error?.let { ErrorCard(it, Modifier.padding(16.dp)) }
            lockedSite?.let { site ->
                ErrorCard(
                    "${vm.label(site)} ist gesperrt.",
                    Modifier.padding(16.dp),
                    title = VaultTexts.LOCKED,
                    actionLabel = VaultTexts.UNLOCK,
                    onAction = { onUnlock(site) },
                )
            }
            val current = head
            if (current == null && error == null && lockedSite == null) {
                Text(VaultTexts.COMPARING, modifier = Modifier.padding(16.dp))
                LinearProgressIndicator(Modifier.fillMaxWidth().padding(horizontal = 16.dp))
            }
            if (current != null) {
                current.unavailable?.let { ErrorCard(it, Modifier.padding(16.dp)) }
                val conflicts = current.counts["conflict"] ?: 0
                if (conflicts > 0) {
                    Row(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedButton(onClick = { askAll = "keep-a" }) { Text(VaultTexts.ALL_CONFLICTS_A) }
                        OutlinedButton(onClick = { askAll = "keep-b" }) { Text(VaultTexts.ALL_CONFLICTS_B) }
                    }
                }
                LazyColumn(Modifier.fillMaxSize().testTag("vault-compare")) {
                    VaultApi.GROUPS.filter { (current.counts[it] ?: 0) > 0 }.forEach { group ->
                        item(key = "head-$group") {
                            GroupHeader(current, group, expanded = group in pages, onToggle = {
                                if (group in pages) pages.remove(group) else scope.launch { loadMore(group) }
                            }, onTransfer = {
                                scope.launch { transferGroup(current, group); round++ }
                            })
                        }
                        val page = pages[group] ?: return@forEach
                        items(page.items, key = { "$group-${it.item}" }) { entry ->
                            EntryRow(entry, onOpen = { detail = entry })
                            if (entry == page.items.last() && page.next != null) {
                                LaunchedEffect(entry.item) { loadMore(group) }
                            }
                        }
                    }
                }
            }
        }
    }
    askAll?.let { action ->
        val count = head?.counts?.get("conflict") ?: 0
        ConfirmDialog(
            title = if (action == "keep-a") VaultTexts.ALL_CONFLICTS_A else VaultTexts.ALL_CONFLICTS_B,
            message = "Für alle $count Konflikte die Fassung ${if (action == "keep-a") "A" else "B"} behalten?",
            confirmLabel = VaultTexts.actionTitle(action),
            onConfirm = {
                askAll = null
                scope.launch {
                    try {
                        val result = VaultApi.resolveAll(jobId, action)
                        if (result.failed.isNotEmpty()) Snackbars.show("${result.failed.size} Konflikte nicht aufgelöst.")
                    } catch (e: CoreException) {
                        error = VaultTexts.errorText(e.kind, e.message.orEmpty())
                    }
                    round++
                }
            },
            onDismiss = { askAll = null },
        )
    }
}

/** „Jetzt übertragen“ of a group: every entry that offers the transfer. */
private suspend fun transferGroup(head: VaultCompareHead, group: String) {
    try {
        loadAll(head, group).filter { "transfer" in it.actions }.forEach { entry ->
            VaultApi.resolve(head.job, entry.item, "transfer", confirmed = true)
        }
    } catch (e: CoreException) {
        Snackbars.show(VaultTexts.errorText(e.kind, e.message.orEmpty()))
    }
}

@Composable
private fun GroupHeader(head: VaultCompareHead, group: String, expanded: Boolean, onToggle: () -> Unit, onTransfer: () -> Unit) {
    Column {
        HorizontalDivider()
        ListItem(
            headlineContent = { Text("${VaultTexts.groupTitle(group, head.labelA, head.labelB)} (${head.counts[group] ?: 0})") },
            trailingContent = {
                if (group in TRANSFER_GROUPS) TextButton(onClick = onTransfer) { Text(VaultTexts.TRANSFER_NOW) }
            },
            supportingContent = { Text(if (expanded) "Einklappen" else "Aufklappen") },
            modifier = Modifier.clickable(onClick = onToggle).testTag("vault-group-$group"),
        )
    }
}

private val TRANSFER_GROUPS = setOf("changed_a", "changed_b", "new_a", "new_b")

@Composable
private fun EntryRow(entry: VaultItem, onOpen: () -> Unit) {
    Card(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 3.dp).clickable(onClick = onOpen)) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(entry.path.ifBlank { entry.finding ?: entry.kind }, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
            Text(
                "A: ${sideText(entry.a.exists, entry.a.size, entry.a.mtimeMs)} · B: ${sideText(entry.b.exists, entry.b.size, entry.b.mtimeMs)}",
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

internal fun sideText(exists: Boolean, size: Long?, mtimeMs: Long?): String {
    if (!exists) return "–"
    val parts = listOfNotNull(size?.let { Format.size(it) }, mtimeMs?.let { Format.dateTime(it) })
    return parts.joinToString(", ").ifBlank { "vorhanden" }
}
