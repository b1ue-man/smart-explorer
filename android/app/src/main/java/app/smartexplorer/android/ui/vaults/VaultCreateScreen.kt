package app.smartexplorer.android.ui.vaults

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.VaultApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.picker.LocationPickerDialog
import kotlinx.coroutines.launch

/** Minimum length of a new password, as Cryptomator (`DEFAULT_MIN_PW_LENGTH`, C7.7). */
internal const val MIN_PASSWORD_CHARS = 8

/**
 * „Neuen Tresor anlegen“ (U31-5): label, location, password twice (at least 8 characters; the
 * core checks it too). Success: the vault is entered and unlocked. `FLAG_SECURE` and no autofill
 * (S3, S6); the password fields are cleared on stop and after every error.
 */
@Composable
fun VaultCreateScreen(onBack: () -> Unit, onCreated: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var label by rememberSaveable { mutableStateOf("") }
    var location by rememberSaveable { mutableStateOf<String?>(null) }
    var picking by rememberSaveable { mutableStateOf(false) }
    val password = remember { mutableStateOf("") }
    val repeat = remember { mutableStateOf("") }
    var working by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    SecureWindow()
    NoAutofill()
    ClearOnStop(password, repeat)
    val long = password.value.codePointCount(0, password.value.length) >= MIN_PASSWORD_CHARS
    val ready = label.isNotBlank() && location != null && long && password.value == repeat.value && !working
    SubPageScaffold(title = VaultTexts.CREATE_NEW, onBack = onBack) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).padding(16.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            error?.let { ErrorCard(it) }
            OutlinedTextField(value = label, onValueChange = { label = it }, label = { Text(VaultTexts.LABEL) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            Text(VaultTexts.CREATE_PLACE, style = MaterialTheme.typography.labelLarge)
            Text(location ?: "–", style = MaterialTheme.typography.bodyMedium)
            OutlinedButton(onClick = { picking = true }, enabled = !working) { Text(VaultTexts.CHOOSE_PLACE) }
            PasswordField(password, VaultTexts.PASSWORD, enabled = !working)
            PasswordField(repeat, VaultTexts.REPEAT_PASSWORD, enabled = !working)
            Text(
                VaultTexts.MIN_LENGTH,
                style = MaterialTheme.typography.bodySmall,
                color = if (long || password.value.isEmpty()) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.error,
            )
            Text(VaultTexts.RECOVERY_HINT, style = MaterialTheme.typography.bodySmall)
            if (working) {
                Text(VaultTexts.CREATING)
                LinearProgressIndicator(Modifier.fillMaxWidth())
            }
            Button(
                enabled = ready,
                onClick = onClick@{
                    val target = location ?: return@onClick
                    val secret = password.value
                    working = true
                    error = null
                    scope.launch {
                        try {
                            val (avail, threshold) = memoryInfo(context)
                            val created = VaultApi.create(target, label, secret, avail, threshold)
                            created.unlockError?.let { Snackbars.show(it) }
                            Snackbars.show(VaultTexts.CREATED)
                            onCreated()
                        } catch (e: CoreException) {
                            error = VaultTexts.errorText(e.kind, e.message.orEmpty(), label)
                        } finally {
                            password.value = ""
                            repeat.value = ""
                            working = false
                        }
                    }
                },
            ) { Text(VaultTexts.CREATE) }
        }
    }
    if (picking) {
        LocationPickerDialog(
            title = VaultTexts.CREATE_PLACE,
            initialLocation = location,
            confirmLabel = VaultTexts.CHOOSE_PLACE,
            onPick = {
                location = it
                picking = false
            },
            onDismiss = { picking = false },
        )
    }
}
