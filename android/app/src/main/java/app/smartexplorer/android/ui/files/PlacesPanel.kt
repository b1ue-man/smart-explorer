package app.smartexplorer.android.ui.files

import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.Roots
import app.smartexplorer.android.core.Root
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.rootIcon

/** At most this many recently visited places (spec F3). */
private const val MAX_RECENT = 10

/** Callbacks of the sidebar "Orte". */
internal class PlacesCallbacks(
    val onOpen: (Root) -> Unit,
    val onFolderSearch: () -> Unit,
    val onRemoveFavorite: (Root) -> Unit,
    /** Connections and Google Drive are set up on the "Mehr" tab. */
    val onManageConnections: () -> Unit,
    val onRetry: () -> Unit,
)

/**
 * Sidebar "Orte" (spec F3), top to bottom: Ordner suchen · Speicher · Favoriten (long press →
 * Entfernen) · Zuletzt · Verbindungen (+ hinzufügen) · Google Drive · Share · Papierkorb.
 */
@Composable
internal fun PlacesPanel(roots: Roots?, error: String?, current: String?, callbacks: PlacesCallbacks) {
    LazyColumn(Modifier.fillMaxSize()) {
        item(key = "search") {
            PlaceItem("Ordner suchen", null, R.drawable.ic_search, selected = false, onClick = callbacks.onFolderSearch)
        }
        if (error != null) {
            item(key = "error") {
                ErrorCard(error, modifier = Modifier.padding(12.dp), title = "Orte nicht geladen", actionLabel = "Erneut", onAction = callbacks.onRetry)
            }
        }
        if (roots == null) return@LazyColumn
        section("Speicher", roots.storage, current, callbacks)
        if (roots.favorites.isNotEmpty()) {
            header("Favoriten")
            items(roots.favorites.distinctBy { it.id }, key = { "fav:${it.id}" }) { root -> FavoriteItem(root, current, callbacks) }
        }
        section("Zuletzt", roots.recent.take(MAX_RECENT), current, callbacks)
        header("Verbindungen")
        items(roots.connections.distinctBy { it.id }, key = { "conn:${it.id}" }) { root -> RootItem(root, current, callbacks) }
        item(key = "conn:add") {
            PlaceItem("Verbindung hinzufügen", null, R.drawable.ic_add, selected = false, onClick = callbacks.onManageConnections)
        }
        header("Google Drive")
        val drive = roots.gdrive
        item(key = "gdrive") {
            if (drive != null) {
                RootItem(drive, current, callbacks)
            } else {
                PlaceItem("Google Drive", "Nicht eingerichtet – unter Mehr einrichten", R.drawable.ic_drive, false, callbacks.onManageConnections)
            }
        }
        section("Share", roots.devices + roots.rooms, current, callbacks)
        roots.trash?.let { trash ->
            item(key = "trash-divider") { HorizontalDivider(Modifier.padding(vertical = 8.dp)) }
            item(key = "trash") { RootItem(trash, current, callbacks) }
        }
    }
}

private fun LazyListScope.header(title: String) {
    item(key = "h:$title") {
        Text(
            title,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 28.dp, top = 16.dp, bottom = 4.dp),
        )
    }
}

private fun LazyListScope.section(title: String, list: List<Root>, current: String?, callbacks: PlacesCallbacks) {
    if (list.isEmpty()) return
    header(title)
    items(list.distinctBy { it.id }, key = { "$title:${it.id}" }) { root -> RootItem(root, current, callbacks) }
}

@Composable
private fun RootItem(root: Root, current: String?, callbacks: PlacesCallbacks) {
    PlaceItem(root.label, root.subtitle, rootIcon(root), selected = root.location == current, onClick = { callbacks.onOpen(root) })
}

@Composable
private fun FavoriteItem(root: Root, current: String?, callbacks: PlacesCallbacks) {
    var menu by remember { mutableStateOf(false) }
    Box {
        PlaceItem(
            root.label,
            root.subtitle,
            rootIcon(root),
            selected = root.location == current,
            onClick = { callbacks.onOpen(root) },
            onLongClick = { menu = true },
        )
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            DropdownMenuItem(
                text = { Text("Entfernen") },
                onClick = {
                    menu = false
                    callbacks.onRemoveFavorite(root)
                },
            )
        }
    }
}

@Composable
private fun PlaceItem(
    label: String,
    subtitle: String?,
    icon: Int,
    selected: Boolean,
    onClick: () -> Unit,
    onLongClick: (() -> Unit)? = null,
) {
    ListItem(
        headlineContent = { Text(label, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = if (subtitle != null) {
            { Text(subtitle, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        } else {
            null
        },
        leadingContent = { SeIcon(icon, contentDescription = null) },
        colors = ListItemDefaults.colors(
            containerColor = if (selected) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent,
        ),
        modifier = Modifier
            .padding(horizontal = 12.dp)
            .combinedClickable(onLongClick = onLongClick, onClick = onClick),
    )
}
