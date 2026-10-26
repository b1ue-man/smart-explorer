package app.smartexplorer.android.ui.login

import android.content.Context
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import app.smartexplorer.android.api.LoginApi
import app.smartexplorer.android.api.LoginTarget
import java.io.ByteArrayOutputStream
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Import is explicit machine pinning. It never writes a normal device grant. */
@Composable
fun LoginTargetDialog(onDismiss: () -> Unit, onOpen: (LoginTarget, String) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var draft by remember { mutableStateOf("") }
    var user by remember { mutableStateOf("") }
    var target by remember { mutableStateOf<LoginTarget?>(null) }
    var acknowledged by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    fun adopt(read: suspend () -> String) {
        if (busy) return
        busy = true; acknowledged = false; target = null; error = null
        scope.launch {
            try { val text = read(); val parsed = LoginApi.importTarget(text); draft = text; target = parsed
            } catch (cancel: CancellationException) { throw cancel
            } catch (failure: Exception) { error = failure.message ?: "Maschinenziel konnte nicht übernommen werden."
            } finally { busy = false }
        }
    }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) adopt { readPublicTarget(context, uri) }
    }
    AlertDialog(onDismissRequest = onDismiss, title = { Text("Administrative Maschinenanmeldung") }, text = {
        Column(Modifier.verticalScroll(rememberScrollState())) {
            Text("gateway-target.json und die freigegebene Ziel-SID/UID vom Administrator übernehmen. Eine normale Geräteidentität oder Bildschirmfreigabe belegt diesen Maschinenzugang nicht.")
            TextButton(enabled = !busy, onClick = { picker.launch(arrayOf("application/json", "text/plain", "application/octet-stream")) }) { Text("gateway-target.json auswählen") }
            OutlinedTextField(value = draft, enabled = !busy, onValueChange = { if (it.toByteArray(Charsets.UTF_8).size <= 32768) { draft = it; target = null; acknowledged = false } },
                label = { Text("Öffentliches Maschinenziel als JSON") }, maxLines = 5)
            TextButton(enabled = !busy && draft.isNotBlank(), onClick = { val text = draft; adopt { text } }) { Text("JSON-Ziel prüfen und übernehmen") }
            target?.let { pin ->
                SelectionContainer { Text("Maschine: ${pin.machineId}\nMaschinenschlüssel: ${pin.publicKey}") }
                Row { Checkbox(checked = acknowledged, enabled = !busy, onCheckedChange = { acknowledged = it }); Text("Diesen administrativ übermittelten Maschinenschlüssel ausdrücklich pinnen") }
            }
            OutlinedTextField(value = user, enabled = !busy, onValueChange = { if (it.length <= 256) user = it }, label = { Text("Freigegebene SID/UID des Zielbenutzers") }, singleLine = true)
            if (busy) Text("Öffentliches Maschinenziel wird geprüft …")
            error?.let { Text(it) }
        }
    }, confirmButton = { TextButton(enabled = target != null && acknowledged && user.isNotBlank() && !busy,
        onClick = { target?.let { onOpen(it, user) } }) { Text("Anmeldung öffnen") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } })
}
private suspend fun readPublicTarget(context: Context, uri: Uri): String = withContext(Dispatchers.IO) {
    context.contentResolver.openInputStream(uri).use { input ->
        requireNotNull(input) { "Öffentliches Maschinenziel konnte nicht gelesen werden." }
        val bytes = ByteArrayOutputStream(); val block = ByteArray(1024)
        while (true) { val count = input.read(block); if (count < 0) break; if (count == 0) continue
            require(bytes.size() + count <= 32768) { "Maschinenziel überschreitet 32 KiB." }; bytes.write(block, 0, count) }
        bytes.toString(Charsets.UTF_8.name())
    }
}
