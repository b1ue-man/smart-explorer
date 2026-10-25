// material3: ModalBottomSheet and FilterChip may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)

package app.smartexplorer.android.ui.vaults

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.ModalBottomSheetProperties
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.SecureFlagPolicy
import app.smartexplorer.android.api.VaultApi
import app.smartexplorer.android.api.VaultSite
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.ConfirmDialog
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.SectionHeader
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.more.ToggleSetting
import kotlinx.coroutines.launch

private val IDLE_MINUTES = listOf(5, 10, 15, 30, 60)

/** „Im Hintergrund sperren“ choices (C3.3): minutes, `null` = „Nie“. */
private val BACKGROUND_CHOICES: List<Pair<Int?, String>> = listOf(
    0 to "Sofort",
    1 to "Nach 1 Minute",
    2 to "Nach 2 Minuten",
    5 to "Nach 5 Minuten",
    10 to "Nach 10 Minuten",
    null to "Nie",
)

/**
 * „Eigenschaften“ of a site (U31-6). Shows only labels and settings, so the page itself sets no
 * `FLAG_SECURE`; the password sheet does (S6). Each change is saved at once (`vault.setSettings`).
 */
@Composable
fun VaultSettingsScreen(vm: VaultViewModel, siteId: String, onBack: () -> Unit, onCheck: () -> Unit, onSync: () -> Unit) {
    val site = vm.site(siteId)
    val scope = rememberCoroutineScope()
    var label by rememberSaveable(siteId) { mutableStateOf(site?.label.orEmpty()) }
    var changing by rememberSaveable { mutableStateOf(false) }
    var forgetting by remember { mutableStateOf(false) }
    var removing by remember { mutableStateOf(false) }
    val save: (suspend () -> Unit) -> Unit = { action ->
        scope.launch {
            runCatchingCore { action() }
            vm.reload()
        }
    }
    SubPageScaffold(title = site?.label?.ifBlank { null } ?: VaultTexts.PROPERTIES, onBack = onBack) { padding ->
        if (site == null) return@SubPageScaffold
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState())) {
            SectionHeader(VaultTexts.SECTION_GENERAL)
            Row(Modifier.padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(value = label, onValueChange = { label = it }, label = { Text(VaultTexts.LABEL) }, singleLine = true, modifier = Modifier.weight(1f))
                TextButton(enabled = label.isNotBlank() && label != site.label, onClick = { save { VaultApi.setSettings(site.site, label = label.trim()) } }) {
                    Text(VaultTexts.SAVE)
                }
            }
            Info("${placeText(site)}: ${site.location}")
            if (site.format != null) Info("Format ${site.format}, Verschlüsselung ${site.cipher ?: "–"}")

            VaultOpenPolicy(site.site, onChanged = { vm.reload() })

            SectionHeader(VaultTexts.SECTION_LOCK)
            ToggleSetting(VaultTexts.LOCK_IDLE, site.autoLockIdleMin != null, onChange = { on ->
                save { VaultApi.setSettings(site.site, setIdle = true, autoLockIdleMin = if (on) 30 else null) }
            })
            site.autoLockIdleMin?.let { current ->
                Choices(VaultTexts.LOCK_IDLE_MINUTES, IDLE_MINUTES.map { it to "$it Minuten" }, current) { minutes ->
                    save { VaultApi.setSettings(site.site, setIdle = true, autoLockIdleMin = minutes) }
                }
            }
            ToggleSetting(VaultTexts.LOCK_SCREEN, site.lockOnScreenLock, onChange = { on ->
                save { VaultApi.setSettings(site.site, lockOnScreenLock = on) }
            })
            Choices(VaultTexts.LOCK_BACKGROUND, BACKGROUND_CHOICES, site.lockOnBackgroundMin) { minutes ->
                save { VaultApi.setSettings(site.site, setBackground = true, lockOnBackgroundMin = minutes) }
            }

            SectionHeader(VaultTexts.SECTION_PASSWORD)
            ActionRow(VaultTexts.CHANGE_PASSWORD) { changing = true }
            if (site.rememberPassword) ActionRow(VaultTexts.FORGET) { forgetting = true }

            SectionHeader(VaultTexts.SECTION_RECOVERY)
            Info(site.recoveryKeyNote ?: VaultTexts.RECOVERY_HINT)

            SectionHeader(VaultTexts.SECTION_SYNC)
            site.syncJobs.forEach { job -> Info(job) }
            ActionRow(VaultTexts.SYNC_SETUP, onSync)

            if (site.unlocked) {
                SectionHeader(VaultTexts.CHECK)
                ActionRow(VaultTexts.CHECK, onCheck)
            }
            OutlinedButton(onClick = { removing = true }, modifier = Modifier.padding(16.dp)) {
                Text(VaultTexts.REMOVE_BUTTON, color = MaterialTheme.colorScheme.error)
            }
        }
    }
    if (changing && site != null) PasswordSheet(site, onDone = { changing = false; vm.reload() })
    if (forgetting && site != null) {
        ConfirmDialog(
            title = VaultTexts.FORGET,
            message = VaultTexts.FORGET_TEXT,
            confirmLabel = VaultTexts.FORGET,
            destructive = true,
            onConfirm = {
                forgetting = false
                save { VaultApi.forgetPassword(site.site) }
            },
            onDismiss = { forgetting = false },
        )
    }
    if (removing && site != null) RemoveDialog(site, onDone = {
        removing = false
        vm.reload()
        onBack()
    })
}

@Composable
private fun Info(text: String) {
    Text(text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
}

@Composable
private fun ActionRow(label: String, onClick: () -> Unit) {
    ListItem(headlineContent = { Text(label) }, modifier = Modifier.clickable(onClick = onClick))
}

@Composable
private fun <T> Choices(title: String, choices: List<Pair<T, String>>, current: T, onChoose: (T) -> Unit) {
    Column(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
        Text(title, style = MaterialTheme.typography.bodyLarge)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            choices.forEach { (value, text) ->
                FilterChip(selected = value == current, onClick = { if (value != current) onChoose(value) }, label = { Text(text) })
            }
        }
    }
}

/** „Passwort ändern“ (U31-6, T10): `FLAG_SECURE`, no autofill, fields cleared on stop. */
@Composable
private fun PasswordSheet(site: VaultSite, onDone: () -> Unit) {
    val scope = rememberCoroutineScope()
    val old = remember { mutableStateOf("") }
    val new = remember { mutableStateOf("") }
    val repeat = remember { mutableStateOf("") }
    var working by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    ClearOnStop(old, new, repeat)
    val long = new.value.codePointCount(0, new.value.length) >= MIN_PASSWORD_CHARS
    ModalBottomSheet(
        onDismissRequest = { if (!working) onDone() },
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        properties = ModalBottomSheetProperties(securePolicy = SecureFlagPolicy.SecureOn),
    ) {
        NoAutofill()
        Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(bottom = 24.dp).navigationBarsPadding(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(VaultTexts.CHANGE_PASSWORD, style = MaterialTheme.typography.titleLarge)
            PasswordField(old, VaultTexts.CURRENT_PASSWORD, enabled = !working)
            PasswordField(new, VaultTexts.NEW_PASSWORD, enabled = !working)
            PasswordField(repeat, VaultTexts.NEW_PASSWORD_REPEAT, enabled = !working)
            Text(VaultTexts.MIN_LENGTH, style = MaterialTheme.typography.bodySmall, color = if (long || new.value.isEmpty()) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.error)
            Text(VaultTexts.PASSWORD_KEY_SAME, style = MaterialTheme.typography.bodySmall)
            error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            if (working) LinearProgressIndicator(Modifier.fillMaxWidth())
            Button(
                enabled = !working && old.value.isNotEmpty() && long && new.value == repeat.value,
                onClick = {
                    val (current, next) = old.value to new.value
                    working = true
                    scope.launch {
                        try {
                            val changed = VaultApi.changePassword(site.site, current, next)
                            Snackbars.show(if (changed.rememberCleared) VaultTexts.REMEMBER_CLEARED else VaultTexts.PASSWORD_CHANGED)
                            onDone()
                        } catch (e: CoreException) {
                            error = VaultTexts.errorText(e.kind, e.message.orEmpty(), site.label)
                        } finally {
                            old.value = ""
                            new.value = ""
                            repeat.value = ""
                            working = false
                        }
                    }
                },
            ) { Text(VaultTexts.CHANGE) }
        }
    }
}
