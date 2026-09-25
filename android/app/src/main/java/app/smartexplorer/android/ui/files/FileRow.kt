package app.smartexplorer.android.ui.files

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.system.Thumbnails
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.kindIcon

/** What a row may offer; decided by the listed place (read-only, local). */
internal data class RowPermissions(val canModify: Boolean, val canCut: Boolean)

/**
 * One row (spec F4): symbol or preview · name (problematic names in the warning color with ⚠) ·
 * "size · date" or "Ordner" · ⋮. In the recursive view rows are indented by depth and folders
 * carry a collapse arrow.
 */
@Composable
internal fun FileRow(
    entry: Entry,
    selected: Boolean,
    selectionMode: Boolean,
    compact: Boolean,
    thumbnail: Boolean,
    tree: Boolean,
    permissions: RowPermissions,
    onClick: () -> Unit,
    onLongClick: () -> Unit,
    onToggleCollapse: () -> Unit,
    onAction: (EntryAction) -> Unit,
) {
    val iconSize = if (compact) 24.dp else 40.dp
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(if (compact) 44.dp else 60.dp)
            .background(if (selected) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surface)
            .combinedClickable(onLongClickLabel = "Auswählen", onLongClick = onLongClick, onClick = onClick)
            .padding(start = 8.dp + if (tree) (entry.depth * 16).dp else 0.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (tree) {
            Box(Modifier.size(32.dp), contentAlignment = Alignment.Center) {
                if (entry.isDir && entry.hasChildren) {
                    Box(
                        modifier = Modifier.size(32.dp).clickable(onClick = onToggleCollapse),
                        contentAlignment = Alignment.Center,
                    ) {
                        SeIcon(
                            if (entry.expanded) R.drawable.ic_expand_more else R.drawable.ic_chevron_right,
                            contentDescription = if (entry.expanded) "Einklappen" else "Ausklappen",
                        )
                    }
                }
            }
        }
        Box(Modifier.padding(horizontal = 8.dp).size(iconSize), contentAlignment = Alignment.Center) {
            if (selected) {
                SeIcon(R.drawable.ic_check, contentDescription = "Ausgewählt", tint = MaterialTheme.colorScheme.primary)
            } else {
                EntryIcon(entry, iconSize, thumbnail)
            }
        }
        Column(Modifier.weight(1f).padding(start = 8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                if (entry.problem != null) {
                    SeIcon(
                        R.drawable.ic_warning,
                        contentDescription = "Problematischer Name",
                        modifier = Modifier.size(16.dp),
                        tint = MaterialTheme.colorScheme.error,
                    )
                }
                Text(
                    entry.name,
                    style = if (compact) MaterialTheme.typography.bodyMedium else MaterialTheme.typography.bodyLarge,
                    color = if (entry.problem != null) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            if (!compact) {
                Text(
                    subtitle(entry),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (compact) {
            Text(
                meta(entry),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                modifier = Modifier.padding(horizontal = 8.dp),
            )
        }
        if (selectionMode) {
            Spacer(Modifier.width(12.dp))
        } else {
            RowMenu(entry, permissions, onAction)
        }
    }
}

/** Placeholder while a window of the recursive view is loading. */
@Composable
internal fun PlaceholderRow(compact: Boolean) {
    Spacer(Modifier.fillMaxWidth().height(if (compact) 44.dp else 60.dp))
}

@Composable
private fun EntryIcon(entry: Entry, size: Dp, thumbnail: Boolean) {
    val tint = if (entry.isDir) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant
    if (!thumbnail || entry.kind != "image") {
        SeIcon(kindIcon(entry.kind), contentDescription = null, modifier = Modifier.size(size * 0.75f), tint = tint)
        return
    }
    val px = with(LocalDensity.current) { size.roundToPx() }.coerceAtLeast(MIN_THUMB_PX)
    val bitmap: ImageBitmap? by produceState(Thumbnails.cached(entry.location, entry.mtimeMs, px), entry.location, entry.mtimeMs, px) {
        // Local entries: the location is the file path (api.md §2).
        if (value == null) value = Thumbnails.load(entry.location, entry.mtimeMs, px)
    }
    val image = bitmap
    if (image != null) {
        Image(
            bitmap = image,
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier = Modifier.size(size).clip(RoundedCornerShape(4.dp)),
        )
    } else {
        SeIcon(kindIcon(entry.kind), contentDescription = null, modifier = Modifier.size(size * 0.75f), tint = tint)
    }
}

/** Row menu "⋮" (spec C): Öffnen mit · Teilen · ── · Kopieren · … · Eigenschaften (+ Entpacken). */
@Composable
private fun RowMenu(entry: Entry, permissions: RowPermissions, onAction: (EntryAction) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { SeIcon(R.drawable.ic_more_vert, contentDescription = "Aktionen für ${entry.name}") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            fun pick(action: EntryAction): () -> Unit = {
                open = false
                onAction(action)
            }
            if (!entry.isDir) {
                MenuEntry("Öffnen mit", enabled = true, onClick = pick(EntryAction.OpenWith))
                MenuEntry("Teilen", enabled = true, onClick = pick(EntryAction.Share))
                HorizontalDivider()
            }
            MenuEntry("Kopieren", enabled = true, onClick = pick(EntryAction.Copy))
            MenuEntry("Ausschneiden", enabled = permissions.canCut, onClick = pick(EntryAction.Cut))
            MenuEntry("Kopieren nach…", enabled = true, onClick = pick(EntryAction.CopyTo))
            MenuEntry("Verschieben nach…", enabled = permissions.canCut, onClick = pick(EntryAction.MoveTo))
            HorizontalDivider()
            MenuEntry("Umbenennen", enabled = permissions.canModify, onClick = pick(EntryAction.Rename))
            MenuEntry("Löschen", enabled = permissions.canModify, onClick = pick(EntryAction.Delete))
            MenuEntry("Eigenschaften", enabled = true, onClick = pick(EntryAction.Properties))
            if (isZip(entry)) MenuEntry("Entpacken", enabled = true, onClick = pick(EntryAction.Extract))
        }
    }
}

internal fun isZip(entry: Entry): Boolean = !entry.isDir && entry.ext.removePrefix(".").equals("zip", ignoreCase = true)

private fun subtitle(entry: Entry): String {
    val base = if (entry.isDir) "Ordner" else "${Format.size(entry.size)} · ${Format.date(entry.mtimeMs)}"
    return entry.problem?.let { "$base · $it" } ?: base
}

private fun meta(entry: Entry): String = if (entry.isDir) "Ordner" else Format.size(entry.size)

private const val MIN_THUMB_PX = 96
