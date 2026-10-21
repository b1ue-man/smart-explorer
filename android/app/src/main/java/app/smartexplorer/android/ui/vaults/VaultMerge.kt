// material3: FilterChip may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.vaults

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.VaultTextPair
import app.smartexplorer.android.api.VaultTextRow
import app.smartexplorer.android.ui.more.SubPageScaffold

/** Which sides of a changed row go into the result. */
internal data class RowChoice(val left: Boolean, val right: Boolean)

/** Defaults as the core's `linemerge`: keep the side with content, A when both differ. */
internal fun defaultChoice(row: VaultTextRow): RowChoice = RowChoice(left = row.left != null, right = row.left == null && row.right != null)

/** The merged text of [pair] with [choices]; line endings and the final newline of version A. */
internal fun assemble(pair: VaultTextPair, choices: List<RowChoice>): String {
    val lines = mutableListOf<String>()
    pair.rows.forEachIndexed { index, row ->
        if (row.equal) {
            row.left?.let(lines::add)
        } else {
            val choice = choices.getOrNull(index) ?: defaultChoice(row)
            if (choice.left) row.left?.let(lines::add)
            if (choice.right) row.right?.let(lines::add)
        }
    }
    val text = lines.joinToString(if (pair.crlf) "\r\n" else "\n")
    return if (pair.finalNewline && lines.isNotEmpty()) text + (if (pair.crlf) "\r\n" else "\n") else text
}

/**
 * „Zusammenführen“ of a vault text conflict (U31-9): per changed row A and/or B, then
 * `vault.mergeApply`. The core keeps both originals encrypted (C6.4). No text field and no
 * selection, so nothing of the cleartext reaches the clipboard (S7); `FLAG_SECURE` (S6).
 */
@Composable
fun VaultMergeRows(pair: VaultTextPair, onBack: () -> Unit, onApply: (String) -> Unit) {
    val choices = remember(pair) { mutableStateListOf<RowChoice>().apply { addAll(pair.rows.map(::defaultChoice)) } }
    SecureWindow()
    SubPageScaffold(title = VaultTexts.MERGE, onBack = onBack) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            LazyColumn(Modifier.weight(1f)) {
                itemsIndexed(pair.rows) { index, row ->
                    if (row.equal) {
                        Text("  " + row.left.orEmpty(), fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(horizontal = 16.dp))
                    } else {
                        Column(Modifier.padding(horizontal = 16.dp, vertical = 2.dp)) {
                            row.left?.let { Text("A: $it", fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall) }
                            row.right?.let { Text("B: $it", fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall) }
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                                val choice = choices[index]
                                if (row.left != null) FilterChip(selected = choice.left, onClick = { choices[index] = choice.copy(left = !choice.left) }, label = { Text("A") })
                                if (row.right != null) FilterChip(selected = choice.right, onClick = { choices[index] = choice.copy(right = !choice.right) }, label = { Text("B") })
                            }
                        }
                    }
                }
            }
            Button(onClick = { onApply(assemble(pair, choices)) }, modifier = Modifier.padding(16.dp)) { Text(VaultTexts.MERGE_APPLY) }
        }
    }
}
