package app.smartexplorer.android.ui.connections

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.api.ConnApi
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.GdriveStatus
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.ProjectLinks
import app.smartexplorer.android.ui.more.TextActions
import app.smartexplorer.android.ui.more.rememberSavedTaskId
import kotlinx.coroutines.launch

/** `Task.kind` of `gdrive.signIn` (api.md §2). */
private const val KIND_OAUTH = "oauth"

/**
 * Google Drive (spec F13): own OAuth client ID (+ optional secret) as on the desktop, [Anmelden]
 * in the browser (up to 3 min, [Abbrechen]), status and [Abmelden]. Used on the connections page
 * and in the settings; it does not scroll by itself.
 */
@Composable
fun GoogleDriveSection(modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var status by remember { mutableStateOf<GdriveStatus?>(null) }
    var loadError by remember { mutableStateOf<String?>(null) }
    var reloadKey by remember { mutableIntStateOf(0) }
    var editing by rememberSaveable { mutableStateOf(false) }
    var clientId by rememberSaveable { mutableStateOf("") }
    // The secret is never put into saved UI state.
    var clientSecret by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var signInTask by rememberSavedTaskId()
    var signInFailure by remember { mutableStateOf<String?>(null) }
    val tasks by Core.tasks.collectAsStateWithLifecycle()
    // Also a sign-in started before this section was recreated or from its other page (settings
    // vs. connections): it keeps [Abbrechen] instead of offering a second [Anmelden].
    val running = signInTask ?: tasks.lastOrNull { it.kind == KIND_OAUTH && it.isActive }?.id

    LaunchedEffect(reloadKey) {
        try {
            val loaded = ConnApi.gdriveStatus()
            status = loaded
            if (clientId.isBlank()) clientId = loaded.clientId.orEmpty()
            loadError = null
        } catch (e: CoreException) {
            loadError = e.message ?: e.kind
        }
    }
    LaunchedEffect(running) {
        val id = running ?: return@LaunchedEffect
        val finished = try {
            FilesApi.awaitTask(id)
        } catch (e: CoreException) {
            signInFailure = e.message ?: e.kind
            null
        }
        signInTask = null
        when (finished?.state) {
            null, "canceled" -> Unit
            "done" -> Snackbars.show("Bei Google Drive angemeldet")
            else -> signInFailure = finished?.message?.takeIf { it.isNotBlank() } ?: "Anmeldung fehlgeschlagen"
        }
        reloadKey++
    }

    fun action(failure: String, block: suspend () -> Unit) {
        scope.launch {
            busy = true
            try {
                block()
            } catch (e: CoreException) {
                Snackbars.show("$failure: ${e.message ?: e.kind}")
            } finally {
                busy = false
                reloadKey++
            }
        }
    }

    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        val current = status
        val error = loadError
        LoadingBar(busy || (current == null && error == null))
        when {
            error != null -> ErrorCard(
                error,
                title = "Google-Drive-Status nicht geladen",
                actionLabel = "Erneut",
                onAction = { reloadKey++ },
            )
            current == null -> Unit
            running != null -> {
                Text("Warte auf die Anmeldung im Browser (bis zu 3 Minuten) …", style = MaterialTheme.typography.bodyMedium)
                OutlinedButton(onClick = { action("Nicht abgebrochen") { FilesApi.cancelTask(running) } }) { Text("Abbrechen") }
            }
            current.signedIn -> {
                Text("Angemeldet", style = MaterialTheme.typography.bodyLarge)
                OutlinedButton(onClick = { action("Nicht abgemeldet") { ConnApi.gdriveSignOut() } }, enabled = !busy) { Text("Abmelden") }
            }
            current.clientConfigured && !editing -> {
                Text("Client-ID hinterlegt, nicht angemeldet.", style = MaterialTheme.typography.bodyMedium)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = {
                        signInFailure = null
                        action("Anmeldung nicht gestartet") { signInTask = ConnApi.gdriveSignIn() }
                    }, enabled = !busy) { Text("Anmelden") }
                    OutlinedButton(onClick = { editing = true }) { Text("Client-ID ändern") }
                }
            }
            else -> {
                OutlinedTextField(
                    value = clientId,
                    onValueChange = { clientId = it },
                    label = { Text("Client-ID") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = clientSecret,
                    onValueChange = { clientSecret = it },
                    label = { Text("Client-Secret (optional)") },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(
                        onClick = {
                            val id = clientId.trim()
                            val secret = clientSecret.trim().ifBlank { null }
                            action("Nicht gespeichert") {
                                ConnApi.gdriveConfigure(id, secret)
                                clientSecret = ""
                                editing = false
                            }
                        },
                        enabled = !busy && clientId.isNotBlank(),
                    ) { Text("Speichern") }
                    if (editing) OutlinedButton(onClick = { editing = false }) { Text("Abbrechen") }
                }
            }
        }
        signInFailure?.let { failure ->
            Text(failure, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error)
        }
        Text(
            "Eigene OAuth-Client-ID wie am Desktop. Die Anmeldung aus dem Android-Browser ist nicht für jedes " +
                "Google-Konto geprüft.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        TextButton(onClick = { TextActions.openUrl(context, ProjectLinks.CLOUD_SETUP) }) { Text("Einrichtungsanleitung") }
    }
}
