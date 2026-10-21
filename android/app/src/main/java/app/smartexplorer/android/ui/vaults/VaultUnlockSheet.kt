// material3: ModalBottomSheet may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.vaults

import android.app.ActivityManager
import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.ModalBottomSheetProperties
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.SecureFlagPolicy
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.smartexplorer.android.R
import app.smartexplorer.android.api.VaultApi
import app.smartexplorer.android.api.VaultSite
import app.smartexplorer.android.api.VaultUnlocked
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/** Outlives a closed sheet, so „Abbrechen“ still reaches the core (S4). */
private val CANCEL_SCOPE = CoroutineScope(SupervisorJob() + Dispatchers.IO)

/** `memAvail` and `memThreshold` of the device for the key derivation (C1.3 (c), U31-4). */
internal fun memoryInfo(context: Context): Pair<Long, Long> {
    val info = ActivityManager.MemoryInfo()
    context.getSystemService(ActivityManager::class.java)?.getMemoryInfo(info)
    return info.availMem to info.threshold
}

/**
 * A password field (U31-4, U31-14, S3): the text lives only in [state] (`remember`, never saved
 * state or a view model), no autofill, no autocorrection; cleared on stop by the caller.
 */
@Composable
internal fun PasswordField(state: MutableState<String>, label: String, enabled: Boolean, modifier: Modifier = Modifier, onDone: () -> Unit = {}) {
    var visible by remember { mutableStateOf(false) }
    OutlinedTextField(
        value = state.value,
        onValueChange = { state.value = it },
        label = { Text(label) },
        enabled = enabled,
        singleLine = true,
        visualTransformation = if (visible) VisualTransformation.None else PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false, imeAction = ImeAction.Done),
        keyboardActions = KeyboardActions(onDone = { onDone() }),
        trailingIcon = {
            IconButton(onClick = { visible = !visible }) {
                SeIcon(R.drawable.ic_visibility, contentDescription = if (visible) VaultTexts.HIDE_PASSWORD else VaultTexts.SHOW_PASSWORD)
            }
        },
        modifier = modifier.fillMaxWidth(),
    )
}

/** Clears [fields] when the activity stops (U31-14). */
@Composable
internal fun ClearOnStop(vararg fields: MutableState<String>) {
    val owner = LocalLifecycleOwner.current
    DisposableEffect(owner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) fields.forEach { it.value = "" }
        }
        owner.lifecycle.addObserver(observer)
        onDispose {
            owner.lifecycle.removeObserver(observer)
            fields.forEach { it.value = "" }
        }
    }
}

/**
 * „<Bezeichnung> entsperren“ (U31-4 with S1, S3, S4, S6): `FLAG_SECURE` on the sheet window, no
 * autofill. „Abbrechen“ during the key derivation cancels the task; the core then drops the key.
 */
@Composable
fun VaultUnlockSheet(site: VaultSite, onDismiss: () -> Unit, onUnlocked: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val password = remember { mutableStateOf("") }
    var rememberChoice by remember { mutableStateOf(false) }
    var task by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val focus = remember { FocusRequester() }
    val deviceLock = remember { VaultSavedPassword.available(context) }
    val label = site.label.ifBlank { site.site }
    ClearOnStop(password)
    val close = {
        password.value = ""
        // Not in the sheet's scope: it ends with the sheet, the cancellation must not (S4).
        task?.let { id -> CANCEL_SCOPE.launch { runCatching { VaultApi.cancelTask(id) } } }
        onDismiss()
    }

    suspend fun unlockWith(secret: String, rememberAfter: Boolean) {
        val (avail, threshold) = memoryInfo(context)
        val id = VaultApi.startUnlock(site.site, secret, avail, threshold)
        task = id
        try {
            VaultApi.awaitTask(id, VaultUnlocked.serializer())
            task = null
            if (rememberAfter) rememberPassword(context, site.site, label, secret)
            password.value = ""
            onUnlocked()
            onDismiss()
        } catch (e: CoreException) {
            task = null
            password.value = ""
            if (e.kind == "canceled") return
            error = VaultTexts.errorText(e.kind, e.message.orEmpty(), label)
            runCatching { focus.requestFocus() }
        }
    }

    ModalBottomSheet(
        onDismissRequest = close,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        properties = ModalBottomSheetProperties(securePolicy = SecureFlagPolicy.SecureOn),
    ) {
        NoAutofill()
        Column(
            Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(bottom = 24.dp).navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("$label entsperren", style = MaterialTheme.typography.titleLarge)
            val busy = task != null
            PasswordField(password, VaultTexts.PASSWORD, enabled = !busy, modifier = Modifier.focusRequester(focus).testTag("vault-password"))
            if (site.rememberPassword) {
                OutlinedButton(
                    enabled = !busy,
                    onClick = {
                        scope.launch {
                            unlockSaved(context, site, label) { unlockWith(it, rememberAfter = false) }?.let { error = it }
                        }
                    },
                ) { Text(VaultTexts.UNLOCK_SAVED) }
            } else {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(checked = rememberChoice && deviceLock, onCheckedChange = { rememberChoice = it }, enabled = deviceLock && !busy)
                    Column {
                        Text(VaultTexts.REMEMBER)
                        if (!deviceLock) Text(VaultTexts.REMEMBER_NEEDS_LOCK, style = MaterialTheme.typography.bodySmall)
                    }
                }
            }
            error?.let { Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("vault-unlock-error")) }
            if (busy) {
                Text(VaultTexts.UNLOCKING)
                LinearProgressIndicator(Modifier.fillMaxWidth())
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    enabled = !busy && password.value.isNotEmpty(),
                    onClick = {
                        error = null
                        val secret = password.value
                        scope.launch { unlockWith(secret, rememberAfter = rememberChoice && deviceLock) }
                    },
                    modifier = Modifier.testTag("vault-unlock"),
                ) { Text(VaultTexts.UNLOCK) }
                TextButton(onClick = close) { Text(VaultTexts.CANCEL) }
            }
        }
    }
}

/** Seals [password] after the device lock confirmed it and hands the envelope to the core (S1). */
private suspend fun rememberPassword(context: Context, site: String, label: String, password: String) {
    val cipher = try {
        VaultSavedPassword.sealCipher()
    } catch (e: Exception) {
        Snackbars.show(VaultTexts.NOT_REMEMBERED)
        return
    }
    when (val confirmed = VaultSavedPassword.confirm(context, VaultTexts.PROMPT_REMEMBER, label, cipher)) {
        is VaultSavedPassword.Confirmed.Ok -> try {
            VaultApi.rememberSaved(site, VaultSavedPassword.seal(confirmed.cipher, site, password))
        } catch (e: CoreException) {
            Snackbars.show(VaultTexts.NOT_REMEMBERED)
        }
        is VaultSavedPassword.Confirmed.Failed -> Snackbars.show(confirmed.text)
        VaultSavedPassword.Confirmed.Dismissed -> Snackbars.show(VaultTexts.NOT_REMEMBERED)
    }
}

/**
 * „Mit gespeichertem Passwort entsperren“ (S1): the envelope from the core, the device lock, then
 * [unlock] with the opened password. Returns the text to show, or `null`.
 */
private suspend fun unlockSaved(context: Context, site: VaultSite, label: String, unlock: suspend (String) -> Unit): String? {
    val sealed = try {
        VaultApi.savedSecret(site.site)
    } catch (e: CoreException) {
        return VaultTexts.errorText(e.kind, e.message.orEmpty(), label)
    }
    val cipher = VaultSavedPassword.openCipher(sealed)
    if (cipher == null) {
        runCatching { VaultApi.forgetPassword(site.site) }
        return VaultTexts.SAVED_INVALID
    }
    return when (val confirmed = VaultSavedPassword.confirm(context, VaultTexts.PROMPT_UNLOCK, label, cipher)) {
        is VaultSavedPassword.Confirmed.Ok -> {
            val secret = VaultSavedPassword.open(confirmed.cipher, site.site, sealed)
            if (secret == null) {
                runCatching { VaultApi.forgetPassword(site.site) }
                VaultTexts.SAVED_INVALID
            } else {
                unlock(secret)
                null
            }
        }
        is VaultSavedPassword.Confirmed.Failed -> confirmed.text
        VaultSavedPassword.Confirmed.Dismissed -> null
    }
}
