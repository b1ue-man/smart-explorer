package app.smartexplorer.android.ui.sync

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import app.smartexplorer.android.api.ConflictSide
import app.smartexplorer.android.ui.common.Format

@Composable
internal fun ConflictVariantChoice(
    label: String, side: ConflictSide?, enabled: Boolean, onChoose: (String?) -> Unit,
) {
    if (side?.needsVariantChoice != true) {
        TextButton(onClick = { onChoose(null) }, enabled = enabled) { Text("$label behalten") }
        return
    }
    var expanded by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { expanded = true }, enabled = enabled) { Text("$label: Version wählen") }
        DropdownMenu(expanded = expanded && enabled, onDismissRequest = { expanded = false }) {
            side.variants.forEachIndexed { index, variant ->
                DropdownMenuItem(
                    text = {
                        Column {
                            Text("Version ${index + 1} · ${Format.size(variant.size)}")
                            Text("${Format.dateTime(variant.mtimeMs)} · Inhalt ${variant.checksum.take(8)}",
                                style = MaterialTheme.typography.bodySmall)
                        }
                    },
                    enabled = variant.id != null,
                    onClick = { expanded = false; onChoose(variant.id) },
                )
            }
        }
    }
}
