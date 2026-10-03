// material3 1.4.0: several components may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.sync

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.MergeChoice
import app.smartexplorer.android.api.MergeRow
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.api.SyncApi.failureText
import app.smartexplorer.android.api.SyncApi.resultAs
import app.smartexplorer.android.api.SyncMergeFailure
import app.smartexplorer.android.api.SyncPendingMerge
import app.smartexplorer.android.api.SyncConflict
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/**
 * Line merge of one text conflict (spec F16, `sync.mergeRows/mergeApply/mergeKeepBoth`).
 * [choices] holds one take-A/take-B pair per row; [onDone] returns to the conflict list.
 */
@Stable
internal class MergeSession(
    val conflicts: ConflictSession,
    val conflict: SyncConflict,
    private val scope: CoroutineScope,
    private val onDone: () -> Unit,
) {
    var loading by mutableStateOf(false)
        private set
    var loadError by mutableStateOf<String?>(null)
        private set
    var rows by mutableStateOf<List<MergeRow>>(emptyList())
        private set
    val choices = mutableStateListOf<MergeChoice>()
    var working by mutableStateOf(false)
        private set
    var requiresCheck by mutableStateOf(false)
        private set
    var retryAllowed by mutableStateOf(false)
        private set
    var pending by mutableStateOf<SyncPendingMerge?>(null)
        private set

    fun load() {
        scope.launch {
            loading = true
            try {
                val loaded = SyncApi.mergeRowsData(conflicts.job.id, conflict.cid)
                rows = loaded.rows
                pending = loaded.pending
                choices.clear()
                choices.addAll(loaded.rows.map { MergeChoice(it.takeA, it.takeB) })
                loadError = null
                requiresCheck = false
                retryAllowed = false
            } catch (e: CoreException) {
                loadError = e.displayText()
            } finally {
                loading = false
            }
        }
    }

    /** [Alle A] / [Alle B]: every differing row takes only that side's line (if it has one). */
    fun takeAll(sideA: Boolean) {
        rows.forEachIndexed { index, row ->
            if (!row.equal) {
                choices[index] = MergeChoice(takeA = sideA && row.a != null, takeB = !sideA && row.b != null)
            }
        }
    }

    fun toggle(index: Int, sideA: Boolean) {
        val current = choices[index]
        choices[index] = if (sideA) current.copy(takeA = !current.takeA) else current.copy(takeB = !current.takeB)
    }

    /** [Übernehmen]: writes the merged text to both sides. */
    fun applyMerge() = finishWith("Zusammengeführt") { SyncApi.mergeApply(conflicts.job.id, conflict.cid, choices.toList()) }

    /** [Beide behalten]: A under the original name, B as "(Konflikt …)" copy on both sides. */
    fun keepBoth() = finishWith("Beide Fassungen behalten") { SyncApi.mergeKeepBoth(conflicts.job.id, conflict.cid) }

    fun retryStored() = finishWith("Zusammenführung abgeschlossen") { SyncApi.mergeRetry(conflicts.job.id, conflict.cid) }

    private fun finishWith(success: String, start: suspend () -> String) {
        if (working || conflicts.finishing) return
        working = true
        // Tracked and marked busy on the conflict list: leaving it waits for this merge before
        // storing the baseline, and the entry cannot be resolved twice meanwhile.
        conflicts.launchTracked {
            conflicts.busy[conflict.key] = true
            try {
                val end = SyncApi.awaitTask(start())
                if (end.state == "done") {
                    conflicts.markResolved(conflict)
                    Snackbars.show(success)
                    onDone()
                } else {
                    val failure = end.failureText()
                    loadError = failure
                    val report = end.resultAs<SyncMergeFailure>()
                    requiresCheck = report?.reload == true
                    retryAllowed = report?.retry == true
                    if (requiresCheck && !retryAllowed) {
                        rows = emptyList()
                        choices.clear()
                        conflicts.load()
                    }
                    Snackbars.show("Nicht zusammengeführt", "Details") { conflicts.showDetails(failure) }
                }
            } catch (e: CoreException) {
                Snackbars.show("Nicht zusammengeführt: ${e.displayText()}")
            } finally {
                conflicts.busy.remove(conflict.key)
                working = false
            }
        }
    }
}

@Composable
internal fun MergeScreen(merge: MergeSession, onBack: () -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text("Zusammenführen", maxLines = 1)
                        Text(
                            ConflictSession.fileName(merge.conflict),
                            style = MaterialTheme.typography.bodySmall,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                },
                navigationIcon = {
                    IconButton(onClick = onBack) { SeIcon(R.drawable.ic_arrow_back, contentDescription = "Zurück") }
                },
            )
        },
        bottomBar = { MergeActions(merge) },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            LoadingBar(merge.loading || merge.working)
            val error = merge.loadError
            if (error != null) {
                ErrorCard(
                    message = error,
                    modifier = Modifier.padding(16.dp),
                    title = if (merge.requiresCheck) "Zusammenführung neu laden" else "Zeilen nicht geladen",
                    actionLabel = if (merge.requiresCheck && !merge.retryAllowed) "Zur Konfliktprüfung" else "Erneut laden",
                    onAction = if (merge.requiresCheck && !merge.retryAllowed) onBack else merge::load,
                )
                return@Column
            }
            val pending = merge.pending
            if (pending != null) {
                LazyColumn(contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    item {
                        Text("Gespeicherten Auftrag fortsetzen", style = MaterialTheme.typography.titleMedium)
                        Text(if (pending.kind == "keep_both")
                            "Beide Fassungen bleiben erhalten. ${if (pending.keepA == true) "A" else "B"} behält den ursprünglichen Namen."
                        else if (pending.preview == null) "Der gespeicherte Dateiinhalt wird auf beiden Seiten übernommen. Eine Textvorschau ist nicht verfügbar."
                        else "Der unten gezeigte zusammengeführte Text wird auf beiden Seiten übernommen.")
                        Text("Bereits bestätigt: A ${if (pending.confirmedA) "ja" else "nein"}, B ${if (pending.confirmedB) "ja" else "nein"}.")
                        Text("Die ursprünglichen Eingaben und die Wahl bleiben erhalten. Wiederholen prüft beide Seiten erneut und meldet zwischenzeitliche Änderungen.")
                    }
                    pending.preview?.let { preview -> item {
                        Text(preview, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
                        if (pending.previewTruncated) Text("Vorschau gekürzt; wiederholt wird der vollständige gespeicherte Text.")
                    } }
                }
                return@Column
            }
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    "${merge.rows.count { !it.equal }} geänderte Zeilen",
                    style = MaterialTheme.typography.labelLarge,
                    modifier = Modifier.weight(1f),
                )
                OutlinedButton(onClick = { merge.takeAll(true) }, enabled = !merge.working) { Text("Alle A") }
                OutlinedButton(onClick = { merge.takeAll(false) }, enabled = !merge.working) { Text("Alle B") }
            }
            HorizontalDivider()
            LazyColumn(contentPadding = PaddingValues(vertical = 4.dp)) {
                itemsIndexed(merge.rows) { index, row ->
                    val choice = merge.choices.getOrNull(index) ?: return@itemsIndexed
                    MergeLine(row, choice, enabled = !merge.working) { sideA -> merge.toggle(index, sideA) }
                }
            }
        }
    }
}

@Composable
private fun MergeActions(merge: MergeSession) {
    Surface(tonalElevation = 3.dp) {
        Row(
            Modifier.fillMaxWidth().navigationBarsPadding().padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
        ) {
            val ready = !merge.working && !merge.loading && merge.loadError == null
            if (merge.pending != null) {
                Button(onClick = merge::retryStored, enabled = ready) { Text("Gespeicherten Auftrag wiederholen") }
            } else {
                TextButton(onClick = merge::keepBoth, enabled = ready) { Text("Beide behalten") }
                Button(onClick = merge::applyMerge, enabled = ready && merge.rows.isNotEmpty()) { Text("Übernehmen") }
            }
        }
    }
}

/** One row: equal lines once, otherwise A and B with a chip each to take that line. */
@Composable
private fun MergeLine(row: MergeRow, choice: MergeChoice, enabled: Boolean, onToggle: (Boolean) -> Unit) {
    if (row.equal) {
        Text(
            row.a ?: row.b.orEmpty(),
            style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 2.dp),
        )
        return
    }
    Column(
        Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .padding(horizontal = 16.dp, vertical = 4.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        row.a?.let { SideLine("A", it, choice.takeA, enabled) { onToggle(true) } }
        row.b?.let { SideLine("B", it, choice.takeB, enabled) { onToggle(false) } }
    }
    HorizontalDivider()
}

@Composable
private fun SideLine(side: String, text: String, selected: Boolean, enabled: Boolean, onToggle: () -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        FilterChip(selected = selected, onClick = onToggle, label = { Text(side) }, enabled = enabled)
        Text(
            text,
            style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
            modifier = Modifier.weight(1f),
        )
    }
}
