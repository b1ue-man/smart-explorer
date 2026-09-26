package app.smartexplorer.android.ui.files

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.NameCheck
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.TaskInfo
import app.smartexplorer.android.ui.common.Format
import kotlinx.coroutines.delay

/**
 * Name dialog for rename / new folder / new file (spec F7): the name is preselected without its
 * extension; an invalid name shows an error and locks OK; an existing name reads "Existiert
 * bereits". With a known [parent] the core checks the name while typing (`fs.checkName`); its
 * warning for problematic names is shown but does not block. [siblings] are the exact names next
 * to a renamed entry.
 */
@Composable
internal fun NameDialog(
    kind: NameDialogKind,
    parent: String?,
    initialName: String,
    onDismiss: () -> Unit,
    onConfirm: (String) -> Unit,
    isDir: Boolean = kind == NameDialogKind.NewFolder,
    siblings: Set<String> = emptySet(),
) {
    val baseEnd = initialName.lastIndexOf('.').takeIf { !isDir && kind == NameDialogKind.Rename && it > 0 } ?: initialName.length
    var value by remember { mutableStateOf(TextFieldValue(initialName, TextRange(0, baseEnd))) }
    var check by remember { mutableStateOf<NameCheck?>(null) }
    // Checked and confirmed alike: blanks around the name (e.g. after a keyboard suggestion) go.
    val name = value.text.trim()
    val localError = localNameError(name)
    val unchanged = kind == NameDialogKind.Rename && name == initialName
    // Case-only renames are allowed even where the file system ignores case (the match is the
    // entry itself), but not onto an exactly named sibling (case-sensitive places).
    val sameAsBefore = kind == NameDialogKind.Rename && name.equals(initialName, ignoreCase = true) && name !in siblings
    LaunchedEffect(name, parent) {
        check = null
        if (parent == null || localError != null || unchanged) return@LaunchedEffect
        delay(CHECK_DEBOUNCE_MS)
        check = try {
            FilesApi.checkName(parent, name)
        } catch (e: CoreException) {
            // Shown as warning; the operation itself reports the definitive error.
            NameCheck(problem = e.message ?: e.kind)
        }
    }
    val exists = check?.exists == true && !sameAsBefore
    val error = localError ?: if (exists) "Existiert bereits" else null
    val canConfirm = error == null && !unchanged
    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Text(
                when (kind) {
                    NameDialogKind.Rename -> "Umbenennen"
                    NameDialogKind.NewFolder -> "Neuer Ordner"
                    NameDialogKind.NewFile -> "Neue Datei"
                },
            )
        },
        text = {
            val focus = remember { FocusRequester() }
            LaunchedEffect(Unit) { focus.requestFocus() }
            val warning = check?.problem
            OutlinedTextField(
                value = value,
                onValueChange = { value = it },
                label = { Text("Name") },
                singleLine = true,
                isError = error != null,
                supportingText = when {
                    error != null -> {
                        { Text(error) }
                    }
                    warning != null -> {
                        { Text("⚠ $warning") }
                    }
                    else -> null
                },
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { if (canConfirm) onConfirm(name) }),
                modifier = Modifier.fillMaxWidth().focusRequester(focus),
            )
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(name) }, enabled = canConfirm) {
                Text(if (kind == NameDialogKind.Rename) "OK" else "Anlegen")
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } },
    )
}

/** Names the file systems never accept; the core warns about further problematic names. */
private fun localNameError(name: String): String? = when {
    name.isBlank() -> "Name fehlt"
    '/' in name || '\u0000' in name -> "„/“ ist in Namen nicht erlaubt"
    name.trim() == "." || name.trim() == ".." -> "Ungültiger Name"
    else -> null
}

private const val CHECK_DEBOUNCE_MS = 250L

/**
 * "n Elemente in den Papierkorb?" with a checkbox "Endgültig löschen"; places without a trash
 * only offer permanent deletion (spec F7). [folderCount] folders selected in a filtered recursive
 * view stand for their whole content: they go only when ticked ([countWithFolders] elements then).
 */
@Composable
internal fun DeleteDialog(
    count: Int,
    canTrash: Boolean,
    onConfirm: (permanent: Boolean, withFolders: Boolean) -> Unit,
    onDismiss: () -> Unit,
    folderCount: Int = 0,
    countWithFolders: Int = count,
) {
    var permanent by remember { mutableStateOf(!canTrash) }
    var withFolders by remember { mutableStateOf(false) }
    val total = if (withFolders) countWithFolders else count
    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Text(
                when {
                    total == 0 -> "Nur Ordner ausgewählt"
                    canTrash -> "${elements(total)} in den Papierkorb?"
                    else -> "${elements(total)} endgültig löschen?"
                },
            )
        },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (folderCount > 0) {
                    Text("Ausgewählte Ordner stehen für ihren gesamten Inhalt – auch für Dateien, die nicht zum Filter passen.")
                    CheckLine(
                        if (folderCount == 1) "Auch den Ordner samt Inhalt löschen" else "Auch die $folderCount Ordner samt Inhalt löschen",
                        withFolders,
                    ) { withFolders = it }
                }
                if (canTrash) {
                    CheckLine("Endgültig löschen", permanent) { permanent = it }
                } else {
                    Text("Dieser Ort hat keinen Papierkorb. Das Löschen lässt sich nicht rückgängig machen.")
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onConfirm(permanent, withFolders) },
                enabled = total > 0,
                colors = if (permanent) ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error) else ButtonDefaults.textButtonColors(),
            ) { Text("Löschen") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } },
    )
}

@Composable
private fun CheckLine(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth().toggleable(value = checked, onValueChange = onChange, role = Role.Checkbox),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = checked, onCheckedChange = null)
        Text(label, modifier = Modifier.padding(start = 8.dp))
    }
}

/**
 * Name conflicts on paste/copy: local→local offers skip / replace / keep both (for all);
 * remote targets always number existing names (desktop rule).
 */
@Composable
internal fun ConflictDialog(names: List<String>, choosable: Boolean, onChoice: (conflict: String) -> Unit, onDismiss: () -> Unit) {
    val shown = names.take(MAX_NAMES).joinToString(", ") + if (names.size > MAX_NAMES) " …" else ""
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (names.size == 1) "Name vorhanden" else "${names.size} Namen vorhanden") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Im Ziel gibt es schon: $shown")
                if (choosable) {
                    Text("Die Wahl gilt für alle.", style = MaterialTheme.typography.bodySmall)
                    ChoiceButton("Überspringen") { onChoice("skip") }
                    ChoiceButton("Ersetzen") { onChoice("replace") }
                    ChoiceButton("Beide behalten") { onChoice("keepBoth") }
                } else {
                    Text("Vorhandene Namen werden nummeriert („Name (2)“).")
                }
            }
        },
        confirmButton = {
            if (choosable) {
                TextButton(onClick = onDismiss) { Text("Abbrechen") }
            } else {
                TextButton(onClick = { onChoice("keepBoth") }) { Text("Einfügen") }
            }
        },
        dismissButton = if (choosable) null else {
            { TextButton(onClick = onDismiss) { Text("Abbrechen") } }
        },
    )
}

private const val MAX_NAMES = 5

@Composable
private fun ChoiceButton(label: String, onClick: () -> Unit) {
    OutlinedButton(onClick = onClick, modifier = Modifier.fillMaxWidth()) { Text(label) }
}

/** "Entpacken": here (folder next to the archive) or elsewhere (spec F10). */
@Composable
internal fun ExtractDialog(name: String, onHere: () -> Unit, onElsewhere: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("„$name“ entpacken") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                ChoiceButton("Hierher", onHere)
                ChoiceButton("Nach…", onElsewhere)
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } },
    )
}

/** Remote file changed since it was opened, like on the desktop (spec F9). */
@Composable
internal fun EditConflictDialog(
    name: String,
    onOverwrite: () -> Unit,
    onCopy: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Konflikt") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("„$name“ wurde seit dem Öffnen auch auf der Gegenseite geändert.")
                OutlinedButton(
                    onClick = onOverwrite,
                    colors = ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.error),
                    modifier = Modifier.fillMaxWidth(),
                ) { Text("Remote überschreiben") }
                ChoiceButton("Als Kopie hochladen", onCopy)
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } },
    )
}

/**
 * "Wird geladen…" while a remote file is downloaded for opening; appears after a short moment and
 * offers [Abbrechen] after one second (spec F9). Back cancels too.
 */
@Composable
internal fun OpenProgressDialog(name: String, task: TaskInfo?, onCancel: () -> Unit) {
    var visible by remember { mutableStateOf(false) }
    var cancellable by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        delay(SHOW_DELAY_MS)
        visible = true
        delay(CANCEL_DELAY_MS - SHOW_DELAY_MS)
        cancellable = true
    }
    if (!visible) return
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text("Wird geladen…") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(name)
                val fraction = task?.let { Format.fraction(it.doneBytes, it.totalBytes) }
                if (fraction != null) {
                    LinearProgressIndicator(progress = { fraction }, modifier = Modifier.fillMaxWidth())
                } else {
                    LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                }
                task?.takeIf { it.totalBytes > 0 }?.let {
                    Text("${Format.size(it.doneBytes)} von ${Format.size(it.totalBytes)}", style = MaterialTheme.typography.bodySmall)
                }
            }
        },
        confirmButton = {
            if (cancellable) TextButton(onClick = onCancel) { Text("Abbrechen") }
        },
    )
}

private const val SHOW_DELAY_MS = 300L
private const val CANCEL_DELAY_MS = 1_000L

/** Read-only text (e.g. scan read problems) with [Kopieren]. */
@Composable
internal fun TextDialog(title: String, text: String, onCopy: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(Modifier.heightIn(max = 400.dp).verticalScroll(rememberScrollState())) {
                Text(text.ifBlank { "Keine Details vorhanden." }, style = MaterialTheme.typography.bodyMedium)
            }
        },
        confirmButton = { TextButton(onClick = onCopy) { Text("Kopieren") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Schließen") } },
    )
}
