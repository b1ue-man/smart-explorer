package app.smartexplorer.android.launcher.ui

import android.content.ActivityNotFoundException
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.launcher.LauncherViewModel
import app.smartexplorer.android.launcher.wallpaper.Wallpaper

/**
 * Rows other packages add to the settings sheet of the launcher (U35-30). U38 puts „Widgets“ (and, with the
 * private space, its switch) into the sheet itself; this list stays for U40.
 */
object EditSheetExtras {
    val rows: MutableList<@Composable () -> Unit> = mutableListOf()
}

private val ICON_SIZES = listOf("Klein" to 48, "Mittel" to 56, "Groß" to 64)

/**
 * The settings sheet at the bottom edge (U35-22, U35-30), opened by a long press on an empty cell: the symbol size
 * (segments „Klein“ 48 dp, „Mittel“ 56 dp, „Groß“ 64 dp) and „Hintergrund ändern“.
 */
@Composable
fun EditSheet(vm: LauncherViewModel, iconDp: Int, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val profiles by vm.catalog.profiles.collectAsStateWithLifecycle()
    val showPrivate by vm.showPrivateSpace.collectAsStateWithLifecycle()
    Surface(
        modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraLarge,
        tonalElevation = 6.dp,
        shadowElevation = 8.dp,
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Startbildschirm-Einstellungen", style = MaterialTheme.typography.titleMedium)
            Text("Symbolgröße", style = MaterialTheme.typography.labelLarge)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                ICON_SIZES.forEach { (label, dp) ->
                    FilterChip(selected = iconDp == dp, onClick = { vm.setIconDp(dp) }, label = { Text(label) })
                }
            }
            OutlinedButton(
                onClick = {
                    try {
                        context.startActivity(Wallpaper.changeIntent())
                    } catch (_: ActivityNotFoundException) {
                        vm.showToast("Hintergrund ändern ist nicht verfügbar.")
                    }
                },
                modifier = Modifier.fillMaxWidth(),
            ) { Text("Hintergrund ändern") }
            OutlinedButton(onClick = { vm.openWidgetPicker() }, modifier = Modifier.fillMaxWidth()) { Text("Widgets") }
            // Only a device that has a private space needs the switch (U38-14).
            if (profiles.any { it.isPrivate }) {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text("Privaten Bereich in der App-Liste zeigen", Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
                    Switch(checked = showPrivate, onCheckedChange = { vm.setShowPrivateSpace(it) })
                }
            }
            EditSheetExtras.rows.forEach { it() }
            TextButton(onClick = { vm.closeEditSheet() }, modifier = Modifier.align(Alignment.End)) { Text("Fertig") }
        }
    }
}

/**
 * „Verschieben nach“ of the accessibility action (U35-32): page (1 to the number of pages + 1), column and
 * row; the result goes through the same placement rule as a drop.
 */
@Composable
fun MoveDialog(pages: Int, columns: Int, rows: Int, onMove: (page: Int, column: Int, row: Int) -> Unit, onDismiss: () -> Unit) {
    var page by remember { mutableStateOf(1) }
    var column by remember { mutableStateOf("1") }
    var row by remember { mutableStateOf("1") }
    var expanded by remember { mutableStateOf(false) }
    val columnValue = column.toIntOrNull()?.takeIf { it in 1..columns }
    val rowValue = row.toIntOrNull()?.takeIf { it in 1..rows }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Verschieben nach") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Column {
                    OutlinedButton(onClick = { expanded = true }, modifier = Modifier.fillMaxWidth()) { Text("Seite $page") }
                    DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
                        (1..pages + 1).forEach { number ->
                            DropdownMenuItem(
                                text = { Text("Seite $number") },
                                onClick = {
                                    page = number
                                    expanded = false
                                },
                            )
                        }
                    }
                }
                OutlinedTextField(
                    value = column,
                    onValueChange = { column = it.filter(Char::isDigit) },
                    label = { Text("Spalte (1 bis $columns)") },
                    isError = columnValue == null,
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                )
                OutlinedTextField(
                    value = row,
                    onValueChange = { row = it.filter(Char::isDigit) },
                    label = { Text("Zeile (1 bis $rows)") },
                    isError = rowValue == null,
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onMove(page, columnValue ?: 1, rowValue ?: 1) },
                enabled = columnValue != null && rowValue != null,
            ) { Text("Verschieben") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } },
    )
}
