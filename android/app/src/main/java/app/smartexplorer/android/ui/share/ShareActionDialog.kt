package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp

/** The action runs in the ViewModel; leaving composition cannot discard its commit or cancel it. */
internal class ShareActionState {
    var saving by mutableStateOf(false)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var attached = true

    fun submit(vm: ShareViewModel, failure: String, onSaved: () -> Unit, block: suspend () -> Unit) {
        if (saving) return
        saving = true
        error = null
        vm.act(failure, owner = this, onResult = { message ->
            if (attached) {
                saving = false
                error = message
                if (message == null) onSaved()
            }
        }, block = block)
    }
}

@Composable
internal fun rememberShareAction(): ShareActionState {
    val state = remember { ShareActionState() }
    DisposableEffect(state) {
        state.attached = true
        onDispose { state.attached = false }
    }
    return state
}

/** Drafts are kept by the caller until a confirmed commit; retries use the visible draft. */
@Composable
internal fun ShareActionDialog(
    title: String,
    state: ShareActionState,
    onDismiss: () -> Unit,
    confirmLabel: String = "Speichern",
    enabled: Boolean = true,
    onConfirm: () -> Unit,
    content: @Composable () -> Unit,
) {
    AlertDialog(
        onDismissRequest = { if (!state.saving) onDismiss() },
        title = { Text(title) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                content()
                if (state.saving) LinearProgressIndicator(Modifier.fillMaxWidth())
                state.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = {
            TextButton(onClick = onConfirm, enabled = enabled && !state.saving) {
                Text(if (state.error != null) "Erneut versuchen" else confirmLabel)
            }
        },
        dismissButton = { TextButton(onClick = onDismiss, enabled = !state.saving) { Text("Abbrechen") } },
    )
}

/** Entire label is tappable and accessible; no implicit change while merely loading a snapshot. */
@Composable
internal fun ShareChoice(label: String, checked: Boolean, enabled: Boolean = true, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().toggleable(value = checked, enabled = enabled, role = Role.Checkbox, onValueChange = onChange),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = checked, onCheckedChange = null, enabled = enabled)
        Text(label, Modifier.padding(start = 8.dp), style = MaterialTheme.typography.bodyMedium)
    }
}
