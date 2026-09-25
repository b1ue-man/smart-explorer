package app.smartexplorer.android.ui.files

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.Listing
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.ui.common.EmptyState
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.Format
import kotlinx.coroutines.delay

/** Row callbacks of one pane. */
internal class ListCallbacks(
    val onOpen: (Entry, local: Boolean) -> Unit,
    val onAction: (EntryAction, Entry, local: Boolean) -> Unit,
    val onCreateFolder: () -> Unit,
    val onResetFilter: () -> Unit,
    val onPlaceSettings: () -> Unit,
)

/**
 * List area of a pane: pull to refresh, error card, empty state, flat listing or recursive tree
 * (spec F4, F5). Tapping opens, long press starts the selection.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun FileListArea(
    tab: BrowserTab,
    compact: Boolean,
    thumbnails: Boolean,
    callbacks: ListCallbacks,
    modifier: Modifier = Modifier,
) {
    PullToRefreshBox(isRefreshing = tab.refreshing, onRefresh = { tab.pullRefresh() }, modifier = modifier.fillMaxSize()) {
        val error = tab.error
        val listing = tab.listing
        val window = tab.scan
        when {
            error != null -> LazyColumn(Modifier.fillMaxSize()) {
                item {
                    Column {
                        ErrorCard(
                            error.message ?: error.kind,
                            modifier = Modifier.padding(16.dp),
                            title = "Ort nicht erreichbar",
                            actionLabel = "Erneut",
                            onAction = { tab.reload() },
                        )
                        // Login or network problems are fixed in the connection settings.
                        if (error.kind == "auth" || error.kind == "network") {
                            TextButton(onClick = callbacks.onPlaceSettings, modifier = Modifier.padding(horizontal = 16.dp)) {
                                Text("Verbindungen bearbeiten")
                            }
                        }
                    }
                }
            }
            window != null -> ScanList(tab, window, compact, thumbnails, listing, callbacks)
            listing != null -> FlatList(tab, listing, compact, thumbnails, callbacks)
        }
    }
}

@Composable
private fun FlatList(tab: BrowserTab, listing: Listing, compact: Boolean, thumbnails: Boolean, callbacks: ListCallbacks) {
    val entries = remember(listing) { listing.entries.distinctBy { it.location } }
    if (entries.isEmpty()) {
        LazyColumn(Modifier.fillMaxSize()) {
            item {
                if (tab.filter.spec() != null) {
                    EmptyState(
                        R.drawable.ic_filter,
                        "Keine Treffer",
                        modifier = Modifier.fillParentMaxSize(),
                        message = "Kein Eintrag passt zum Filter.",
                        actionLabel = "Filter aufheben",
                        onAction = callbacks.onResetFilter,
                    )
                } else {
                    EmptyState(
                        R.drawable.ic_folder,
                        "Dieser Ordner ist leer",
                        modifier = Modifier.fillParentMaxSize(),
                        actionLabel = if (listing.readOnly) null else "Neuer Ordner",
                        onAction = if (listing.readOnly) null else callbacks.onCreateFolder,
                    )
                }
            }
        }
        return
    }
    val permissions = RowPermissions(canModify = !listing.readOnly, canCut = !listing.readOnly && listing.isLocal)
    val selectionMode = tab.selection.isNotEmpty()
    LazyColumn(state = tab.listState, modifier = Modifier.fillMaxSize()) {
        items(entries, key = { it.location }) { entry ->
            FileRow(
                entry = entry,
                selected = entry.location in tab.selection,
                selectionMode = selectionMode,
                compact = compact,
                thumbnail = thumbnails && listing.isLocal,
                tree = false,
                permissions = permissions,
                onClick = {
                    if (selectionMode) tab.toggleSelection(entry) else callbacks.onOpen(entry, listing.isLocal)
                },
                onLongClick = { tab.toggleSelection(entry) },
                onToggleCollapse = {},
                onAction = { callbacks.onAction(it, entry, listing.isLocal) },
            )
        }
    }
}

@Composable
private fun ScanList(
    tab: BrowserTab,
    window: ScanWindow,
    compact: Boolean,
    thumbnails: Boolean,
    listing: Listing?,
    callbacks: ListCallbacks,
) {
    LaunchedEffect(window) {
        snapshotFlow {
            val visible = tab.listState.layoutInfo.visibleItemsInfo
            (visible.firstOrNull()?.index ?: 0) to (visible.lastOrNull()?.index ?: 0)
        }.collect { (first, last) -> window.onVisible(first, last) }
    }
    val readOnly = listing?.readOnly ?: true
    val permissions = RowPermissions(canModify = !readOnly, canCut = !readOnly && window.local)
    val selectionMode = tab.selection.isNotEmpty()
    if (!window.running && window.total == 0 && window.error == null && window.taskId != null) {
        EmptyState(
            R.drawable.ic_search,
            "Keine Treffer",
            message = "Nichts unterhalb dieses Ordners passt zum Filter.",
            actionLabel = "Filter aufheben",
            onAction = callbacks.onResetFilter,
        )
        return
    }
    // Rows are addressed by position: windows of different revisions may briefly overlap.
    LazyColumn(state = tab.listState, modifier = Modifier.fillMaxSize()) {
        items(count = window.total) { index ->
            val entry = window.entryAt(index)
            if (entry == null) {
                PlaceholderRow(compact)
            } else {
                FileRow(
                    entry = entry,
                    selected = entry.location in tab.selection,
                    selectionMode = selectionMode,
                    compact = compact,
                    thumbnail = thumbnails && window.local,
                    tree = true,
                    permissions = permissions,
                    onClick = {
                        if (selectionMode) tab.toggleSelection(entry) else callbacks.onOpen(entry, window.local)
                    },
                    onLongClick = { tab.toggleSelection(entry) },
                    onToggleCollapse = { tab.toggleCollapsed(entry.location) },
                    onAction = { callbacks.onAction(it, entry, window.local) },
                )
            }
        }
    }
}

/**
 * Footer: "N Elemente · Größe", with a filter "N Treffer", recursive "Treffer · durchsucht" plus
 * progress with [Stopp] (after 1 s), scan limit and read problems with [Details].
 */
@Composable
internal fun ListFooter(tab: BrowserTab, tasks: List<TaskInfo>, onStop: () -> Unit, onIssues: (String) -> Unit) {
    val window = tab.scan
    val listing = tab.listing
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
        if (window != null) {
            ScanFooter(window, tasks, onStop, onIssues, onRescan = { tab.reload() })
        } else if (listing != null && tab.error == null) {
            val count = listing.entries.size
            val label = if (tab.filter.spec() != null) "$count Treffer" else elements(count)
            Text(
                "$label · ${Format.size(listing.totalBytes)}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun ScanFooter(
    window: ScanWindow,
    tasks: List<TaskInfo>,
    onStop: () -> Unit,
    onIssues: (String) -> Unit,
    onRescan: () -> Unit,
) {
    val style = MaterialTheme.typography.bodySmall
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    window.error?.let { message ->
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(message, style = style, color = MaterialTheme.colorScheme.error, modifier = Modifier.weight(1f))
            TextButton(onClick = onRescan) { Text("Neu scannen") }
        }
    }
    val task = tasks.firstOrNull { it.id == window.taskId }
    val scanned = maxOf(window.scanned, task?.doneItems ?: 0)
    val matches = maxOf(window.matches, task?.totalItems ?: 0)
    var slow by remember(window) { mutableStateOf(false) }
    LaunchedEffect(window) {
        delay(SLOW_SCAN_MS)
        slow = true
    }
    if (window.running && slow) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
            Text("$scanned durchsucht", style = style, color = muted, modifier = Modifier.weight(1f))
            TextButton(onClick = onStop) { Text("Stopp") }
        }
    }
    Text("$matches Treffer · $scanned durchsucht", style = style, color = muted)
    if (window.truncated) {
        Text("Scan-Grenze erreicht – Ergebnisse unvollständig", style = style, color = MaterialTheme.colorScheme.error)
    }
    val id = window.taskId
    if (window.issues > 0 && id != null) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("${window.issues} Ordner nicht lesbar", style = style, color = muted, modifier = Modifier.weight(1f))
            TextButton(onClick = { onIssues(id) }) { Text("Details") }
        }
    }
}

private const val SLOW_SCAN_MS = 1_000L
