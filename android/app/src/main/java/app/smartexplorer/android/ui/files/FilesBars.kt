package app.smartexplorer.android.ui.files

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.EditInfo
import app.smartexplorer.android.ui.common.SeIcon

/** "n Elemente kopiert – [Hier einfügen] [✕]", also after changing places (spec F7). */
@Composable
internal fun PasteBar(clip: Clip, canPaste: Boolean, onPaste: () -> Unit, onClear: () -> Unit) {
    Surface(color = MaterialTheme.colorScheme.secondaryContainer, modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier.padding(start = 16.dp, end = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            SeIcon(if (clip.move) R.drawable.ic_cut else R.drawable.ic_copy, contentDescription = null)
            Text(
                "${elements(clip.sources.size)} ${if (clip.move) "ausgeschnitten" else "kopiert"}",
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            TextButton(onClick = onPaste, enabled = canPaste) { Text("Hier einfügen") }
            IconButton(onClick = onClear) { SeIcon(R.drawable.ic_close, contentDescription = "Zwischenablage leeren") }
        }
    }
}

/** "Geänderte Datei: name – [Hochladen] [Verwerfen]" for an edited remote copy (spec F9). */
@Composable
internal fun EditCard(edit: EditInfo, onUpload: () -> Unit, onDiscard: () -> Unit) {
    Card(
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.tertiaryContainer),
        modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp),
    ) {
        Column(Modifier.padding(start = 16.dp, end = 8.dp, top = 12.dp)) {
            Text("Geänderte Datei: ${edit.name}", style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(
                edit.location,
                style = MaterialTheme.typography.bodySmall,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Row(Modifier.align(Alignment.End)) {
                TextButton(onClick = onDiscard) { Text("Verwerfen") }
                TextButton(onClick = onUpload) { Text("Hochladen") }
            }
        }
    }
}

/** Floating (+): "Neuer Ordner" / "Neue Datei" (spec F7). */
@Composable
internal fun NewItemFab(onNew: (folder: Boolean) -> Unit) {
    var menu by remember { mutableStateOf(false) }
    Box {
        FloatingActionButton(onClick = { menu = true }) { SeIcon(R.drawable.ic_add, contentDescription = "Neu") }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            DropdownMenuItem(
                text = { Text("Neuer Ordner") },
                leadingIcon = { SeIcon(R.drawable.ic_folder, contentDescription = null) },
                onClick = {
                    menu = false
                    onNew(true)
                },
            )
            DropdownMenuItem(
                text = { Text("Neue Datei") },
                leadingIcon = { SeIcon(R.drawable.ic_file, contentDescription = null) },
                onClick = {
                    menu = false
                    onNew(false)
                },
            )
        }
    }
}
