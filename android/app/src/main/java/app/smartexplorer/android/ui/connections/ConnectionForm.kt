// material3 1.4.0: SegmentedButton may still be experimental (docs/refs/compose-material3.md §0, §14).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.connections

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.ConnApi
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.more.ToggleSetting

/**
 * Connection form (spec F12): Typ · Name · Host · Port · Benutzer · Passwort or Schlüsseldatei +
 * Passphrase (SFTP) · Startordner · Remote-Agent (SFTP) · [Testen] [Speichern]; WebDAV immer über HTTPS.
 */
@Composable
internal fun ConnectionFormPage(state: ConnectionFormState, onTest: () -> Unit, onSave: () -> Unit, onClose: () -> Unit) {
    val draft = state.draft
    val errors = if (state.showErrors) draft.errors() else emptyMap()
    var pickKey by remember { mutableStateOf(false) }
    val update: (ConnectionDraft) -> Unit = { next ->
        state.draft = next
        state.outcome = null
    }

    SubPageScaffold(title = if (state.isEdit) "Verbindung bearbeiten" else "Neue Verbindung", onBack = onClose) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .imePadding()
                .verticalScroll(rememberScrollState()),
        ) {
            LoadingBar(state.testing || state.saving)
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                ChoiceRow(
                    options = ConnApi.PROTOCOLS.map { it to ConnectionDraft.protocolLabel(it) },
                    selected = draft.protocol,
                    onSelect = { update(draft.withProtocol(it)) },
                )
                FormField("Name", draft.label, { update(draft.copy(label = it)) }, placeholder = draft.host.ifBlank { "Mein Server" })
                FormField("Host", draft.host, { update(draft.copy(host = it)) }, error = errors[ConnField.Host], keyboard = KeyboardType.Uri)
                FormField(
                    "Port",
                    draft.port,
                    { update(draft.copy(port = it.filter(Char::isDigit))) },
                    error = errors[ConnField.Port],
                    keyboard = KeyboardType.Number,
                )
                FormField("Benutzer", draft.user, { update(draft.copy(user = it)) }, keyboard = KeyboardType.Ascii)
                if (draft.isSftp) {
                    ChoiceRow(
                        options = listOf("password" to "Passwort", "key" to "Schlüsseldatei"),
                        selected = if (draft.usesKey) "key" else "password",
                        onSelect = { update(draft.copy(auth = it)) },
                    )
                }
                val unchanged = if (state.isEdit) "Leer = unverändert" else null
                if (draft.usesKey) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        FormField(
                            "Schlüsseldatei",
                            draft.keyPath,
                            { update(draft.copy(keyPath = it)) },
                            error = errors[ConnField.KeyPath],
                            modifier = Modifier.weight(1f),
                        )
                        OutlinedButton(onClick = { pickKey = true }) { Text("Wählen") }
                    }
                    SecretField("Passphrase", state.passphrase, { state.passphrase = it }, placeholder = unchanged)
                } else {
                    SecretField("Passwort", state.password, { state.password = it }, placeholder = unchanged)
                }
            }
            if (draft.isSftp) {
                // Desktop "Remote-Agent verwenden": deploys Smart Explorer's own agent over SFTP for fast
                // server-side operations; it is not an ssh-agent login.
                ToggleSetting("Remote-Agent verwenden", draft.useAgent, { update(draft.copy(useAgent = it)) })
                HintLine("Ohne Remote-Agent ersetzt SFTP keine vorhandenen Dateien (wie am Desktop): Sync-Aktualisierungen und bearbeitete Dateien brauchen ihn.")
                HintLine("Den ersten Hostschlüssel speichert die App (wie am Desktop); ein geänderter Schlüssel lässt die Verbindung scheitern.")
            }
            // WebDAV always uses HTTPS (draft default and saved connections), like the desktop format.
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                FormField("Startordner", draft.root, { update(draft.copy(root = it)) }, keyboard = KeyboardType.Uri)
                state.outcome?.let { outcome ->
                    Text(
                        outcome.text,
                        style = MaterialTheme.typography.bodyMedium,
                        color = if (outcome.ok) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.error,
                    )
                }
                Row(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    OutlinedButton(onClick = onTest, enabled = !state.testing && !state.saving, modifier = Modifier.weight(1f)) {
                        Text("Testen")
                    }
                    Button(onClick = onSave, enabled = !state.testing && !state.saving, modifier = Modifier.weight(1f)) {
                        Text("Speichern")
                    }
                }
            }
        }
    }

    if (pickKey) {
        LocalFilePickerDialog(
            title = "Schlüsseldatei wählen",
            multiple = false,
            showHiddenInitially = true,
            onPick = { paths ->
                pickKey = false
                paths.firstOrNull()?.let { update(state.draft.copy(keyPath = it)) }
            },
            onDismiss = { pickKey = false },
        )
    }
}

/** One-of choice as segmented buttons without check marks (labels must fit a phone width). */
@Composable
private fun ChoiceRow(options: List<Pair<String, String>>, selected: String, onSelect: (String) -> Unit) {
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
        options.forEachIndexed { index, (value, label) ->
            SegmentedButton(
                selected = selected == value,
                onClick = { onSelect(value) },
                shape = SegmentedButtonDefaults.itemShape(index = index, count = options.size),
                icon = {},
                label = { Text(label, maxLines = 1) },
            )
        }
    }
}

@Composable
private fun FormField(
    label: String,
    value: String,
    onChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String? = null,
    error: String? = null,
    keyboard: KeyboardType = KeyboardType.Text,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        placeholder = if (placeholder != null) {
            { Text(placeholder) }
        } else {
            null
        },
        isError = error != null,
        supportingText = if (error != null) {
            { Text(error) }
        } else {
            null
        },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = keyboard),
        modifier = modifier.fillMaxWidth(),
    )
}

@Composable
private fun SecretField(label: String, value: String, onChange: (String) -> Unit, placeholder: String?) {
    var visible by remember { mutableStateOf(false) }
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        placeholder = if (placeholder != null) {
            { Text(placeholder) }
        } else {
            null
        },
        singleLine = true,
        visualTransformation = if (visible) VisualTransformation.None else PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
        trailingIcon = {
            IconButton(onClick = { visible = !visible }) {
                SeIcon(R.drawable.ic_visibility, contentDescription = if (visible) "Verbergen" else "Anzeigen")
            }
        },
        modifier = Modifier.fillMaxWidth(),
    )
}
