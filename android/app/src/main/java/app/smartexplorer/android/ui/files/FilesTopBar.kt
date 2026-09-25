package app.smartexplorer.android.ui.files

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Crumb
import app.smartexplorer.android.ui.common.SeIcon

/** Actions on entries, shared by the row menu and the selection bar (spec C). */
internal enum class EntryAction { OpenWith, Share, Copy, Cut, CopyTo, MoveTo, Rename, Delete, Properties, Extract, CopyPath, Favorite }

/** Menu "⋮" of the title bar (spec C: 9 entries in groups). */
internal enum class FolderAction { ViewOptions, FolderSearch, NewTab, Forward, Mirror, Analyze, Favorite, CopyPath, Properties }

/** Title bar: ☰ · place · 🔍 · [tabs] · ⋮. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun FilesTopBar(
    title: String,
    location: String?,
    showMenuButton: Boolean,
    onMenu: () -> Unit,
    filterOpen: Boolean,
    onToggleFilter: () -> Unit,
    tabCount: Int,
    onTabs: () -> Unit,
    canForward: Boolean,
    canFavorite: Boolean,
    onAction: (FolderAction) -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    // Label of the favorite entry; unknown (null) until checked when the menu opens.
    val favorite by produceState<Boolean?>(null, menu, location) {
        value = if (menu && location != null && canFavorite) {
            try {
                FilesApi.isFavorite(location)
            } catch (e: CoreException) {
                // The toggle itself reports any error; the label just stays neutral.
                null
            }
        } else {
            null
        }
    }
    TopAppBar(
        title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        navigationIcon = {
            if (showMenuButton) {
                IconButton(onClick = onMenu) { SeIcon(R.drawable.ic_menu, contentDescription = "Orte") }
            }
        },
        actions = {
            IconButton(onClick = onToggleFilter) {
                SeIcon(
                    if (filterOpen) R.drawable.ic_close else R.drawable.ic_search,
                    contentDescription = if (filterOpen) "Filter schließen" else "Filtern",
                )
            }
            TabsButton(tabCount, onTabs)
            Box {
                IconButton(onClick = { menu = true }) { SeIcon(R.drawable.ic_more_vert, contentDescription = "Menü") }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    MenuEntry("Ansicht…", enabled = true) { menu = false; onAction(FolderAction.ViewOptions) }
                    MenuEntry("Ordner suchen", enabled = true) { menu = false; onAction(FolderAction.FolderSearch) }
                    HorizontalDivider()
                    MenuEntry("Neuer Tab", enabled = true) { menu = false; onAction(FolderAction.NewTab) }
                    MenuEntry("Vor", enabled = canForward) { menu = false; onAction(FolderAction.Forward) }
                    HorizontalDivider()
                    MenuEntry("Spiegeln nach…", enabled = canFavorite && location != null) { menu = false; onAction(FolderAction.Mirror) }
                    MenuEntry("Analysieren", enabled = canFavorite && location != null) { menu = false; onAction(FolderAction.Analyze) }
                    HorizontalDivider()
                    MenuEntry(
                        if (favorite == true) "Aus Favoriten entfernen" else "Zu Favoriten",
                        enabled = canFavorite && location != null,
                    ) { menu = false; onAction(FolderAction.Favorite) }
                    MenuEntry("Pfad kopieren", enabled = location != null) { menu = false; onAction(FolderAction.CopyPath) }
                    MenuEntry("Eigenschaften", enabled = location != null) { menu = false; onAction(FolderAction.Properties) }
                }
            }
        },
    )
}

/** Title bar in selection mode: ✕ · "n ausgewählt" · ⧉ · ✂ · 🗑 · ⤴ · ⋮ (spec C). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun SelectionTopBar(
    count: Int,
    canModify: Boolean,
    canCut: Boolean,
    canRename: Boolean,
    canExtract: Boolean,
    canFavorite: Boolean,
    onClear: () -> Unit,
    onSelectAll: () -> Unit,
    onInvert: () -> Unit,
    onAction: (EntryAction) -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    TopAppBar(
        title = { Text("$count ausgewählt") },
        navigationIcon = {
            IconButton(onClick = onClear) { SeIcon(R.drawable.ic_close, contentDescription = "Auswahl aufheben") }
        },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.secondaryContainer),
        actions = {
            IconButton(onClick = { onAction(EntryAction.Copy) }) { SeIcon(R.drawable.ic_copy, contentDescription = "Kopieren") }
            IconButton(onClick = { onAction(EntryAction.Cut) }, enabled = canCut) {
                SeIcon(R.drawable.ic_cut, contentDescription = "Ausschneiden")
            }
            IconButton(onClick = { onAction(EntryAction.Delete) }, enabled = canModify) {
                SeIcon(R.drawable.ic_delete, contentDescription = "Löschen")
            }
            IconButton(onClick = { onAction(EntryAction.Share) }) { SeIcon(R.drawable.ic_share, contentDescription = "Teilen") }
            Box {
                IconButton(onClick = { menu = true }) { SeIcon(R.drawable.ic_more_vert, contentDescription = "Weitere Aktionen") }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    MenuEntry("Alle auswählen", enabled = true) { menu = false; onSelectAll() }
                    MenuEntry("Auswahl umkehren", enabled = true) { menu = false; onInvert() }
                    HorizontalDivider()
                    MenuEntry("Umbenennen", enabled = canRename) { menu = false; onAction(EntryAction.Rename) }
                    MenuEntry("Kopieren nach…", enabled = true) { menu = false; onAction(EntryAction.CopyTo) }
                    MenuEntry("Verschieben nach…", enabled = canCut) { menu = false; onAction(EntryAction.MoveTo) }
                    if (canExtract) MenuEntry("Entpacken", enabled = true) { menu = false; onAction(EntryAction.Extract) }
                    HorizontalDivider()
                    MenuEntry("Pfad kopieren", enabled = true) { menu = false; onAction(EntryAction.CopyPath) }
                    MenuEntry("Eigenschaften", enabled = true) { menu = false; onAction(EntryAction.Properties) }
                    MenuEntry("Zu Favoriten", enabled = canFavorite) { menu = false; onAction(EntryAction.Favorite) }
                }
            }
        },
    )
}

@Composable
internal fun MenuEntry(label: String, enabled: Boolean, onClick: () -> Unit) {
    DropdownMenuItem(text = { Text(label) }, onClick = onClick, enabled = enabled)
}

/** Tab switcher button showing the number of tabs (spec C "[2]"). */
@Composable
private fun TabsButton(count: Int, onClick: () -> Unit) {
    IconButton(onClick = onClick, modifier = Modifier.semantics { contentDescription = "Tabs: $count" }) {
        Surface(
            shape = RoundedCornerShape(4.dp),
            border = BorderStroke(1.5.dp, LocalContentColor.current),
            color = Color.Transparent,
        ) {
            Text(
                count.toString(),
                style = MaterialTheme.typography.labelMedium,
                textAlign = TextAlign.Center,
                modifier = Modifier.defaultMinSize(minWidth = 20.dp).padding(horizontal = 4.dp, vertical = 1.dp),
            )
        }
    }
}

/**
 * Path bar (spec F3): horizontally scrollable, every segment tappable, last segment bold; scrolls
 * to the end whenever the path changes.
 */
@Composable
internal fun CrumbBar(crumbs: List<Crumb>, onOpen: (String) -> Unit, modifier: Modifier = Modifier) {
    if (crumbs.isEmpty()) return
    val scroll = rememberScrollState()
    LaunchedEffect(crumbs) { snapshotFlow { scroll.maxValue }.collect { scroll.scrollTo(it) } }
    Row(
        modifier = modifier.fillMaxWidth().horizontalScroll(scroll).padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        crumbs.forEachIndexed { index, crumb ->
            val last = index == crumbs.lastIndex
            if (index > 0) {
                Text("›", color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 2.dp))
            }
            Text(
                crumb.label,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = if (last) FontWeight.Bold else null,
                color = if (last) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.primary,
                maxLines = 1,
                modifier = Modifier
                    .clickable(enabled = !last) { onOpen(crumb.location) }
                    .padding(horizontal = 6.dp, vertical = 10.dp),
            )
        }
    }
}
