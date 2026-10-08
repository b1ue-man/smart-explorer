package app.smartexplorer.android.ui.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
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
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.ShareSecurityApi
import app.smartexplorer.android.api.ShareServerInfo
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SectionHeader
import app.smartexplorer.android.ui.share.ShareChoice
import kotlinx.coroutines.launch

/** ServerInfo is the authority for stored transport and its legacy migration. Drafts retain errors. */
@Composable
internal fun ShareServerSettings() {
    val scope = rememberCoroutineScope()
    var server by rememberSaveable { mutableStateOf("") }
    var draftPresent by rememberSaveable { mutableStateOf(false) }
    var info by remember { mutableStateOf<ShareServerInfo?>(null) }
    var loading by remember { mutableStateOf(true) }
    var saving by remember { mutableStateOf(false) }
    var allowPlaintext by remember { mutableStateOf(false) }
    var loadError by remember { mutableStateOf<String?>(null) }
    var saveError by remember { mutableStateOf<String?>(null) }
    var reloadKey by remember { mutableIntStateOf(0) }
    LaunchedEffect(reloadKey) {
        loading = true
        try {
            val loaded = ShareSecurityApi.serverInfo()
            info = loaded
            // A rotation or retry must not erase an unsaved address or accidentally remove a server.
            if (!draftPresent) {
                server = loaded.server
                draftPresent = true
            }
            loadError = null
        } catch (e: CoreException) {
            loadError = "Share-Server nicht geladen: ${e.message ?: e.kind}"
        } finally {
            loading = false
        }
    }
    val insecureInput = usesPlaintext(server)
    SectionHeader("Share-Server")
    Column(Modifier.padding(horizontal = 16.dp)) {
        info?.let { current ->
            Text(
                when (current.security) {
                    "encrypted" -> "Gespeichert: verschlüsselt · ${current.summary}"
                    "plaintext" -> "⚠ Gespeichert: unverschlüsselt · ${current.summary}"
                    else -> "Gespeichert: nur LAN"
                },
                color = if (current.security == "plaintext") MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (current.migrated) Text("Die alte Adresse bleibt TCP. Für Verschlüsselung eine TLS-Adresse eingeben.")
            current.encryptedAlternative?.let { encrypted ->
                Text(
                    "Verschlüsselt verbinden: Der Server braucht ein TLS-Zertifikat für den eingetragenen Namen " +
                        "(se-share-server mit --tls-cert/--tls-key bzw. SE_SHARE_TLS_CERT/SE_SHARE_TLS_KEY). " +
                        "Selbst signiert: #sha256=<Fingerabdruck> anhängen. Ein TLS-Fehler fällt nie auf Klartext zurück.",
                )
                if (current.namesIpAddress) {
                    Text(
                        "Die Adresse ist eine IP-Adresse: den Servernamen eintragen, auf den das Zertifikat ausgestellt ist.",
                        color = MaterialTheme.colorScheme.error,
                    )
                }
                OutlinedButton(
                    onClick = { server = encrypted; allowPlaintext = false },
                    enabled = !saving && !loading,
                ) { Text("Übernehmen: $encrypted") }
                Text("Danach speichern.")
            }
            if (current.ignoredPlaintext > 0) Text("${current.ignoredPlaintext} alte Klartexteinträge werden unter TLS ignoriert. Kein Rückfall auf Klartext.")
        }
        OutlinedTextField(
            value = server,
            onValueChange = { server = it; allowPlaintext = false },
            label = { Text("Adresse") },
            placeholder = { Text("wss://server.example.org:51820") },
            singleLine = true,
            enabled = info != null && loadError == null && !loading && !saving,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
            modifier = Modifier.fillMaxWidth(),
        )
        Text("Ohne Schema wird TLS verwendet. Selbst signierte Server: Adresse#sha256=<Zertifikats-Fingerabdruck>.")
        if (insecureInput) {
            Text("⚠ TCP/WS/HTTP überträgt die Server-Verbindung unverschlüsselt. TLS und Klartext dürfen bei einer neuen Eingabe nicht gemischt werden.", color = MaterialTheme.colorScheme.error)
            ShareChoice("Unverschlüsselt erlauben", allowPlaintext, !saving && !loading) { allowPlaintext = it }
        }
        loadError?.let { error ->
            Text(error, color = MaterialTheme.colorScheme.error)
            OutlinedButton(onClick = { reloadKey++ }, enabled = !loading) { Text("Erneut laden") }
        }
        saveError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        OutlinedButton(
            onClick = {
                val value = server.trim()
                val optedIn = allowPlaintext
                saving = true
                scope.launch {
                    try {
                        val saved = ShareSecurityApi.setServer(value, optedIn)
                        info = saved
                        server = saved.server
                        saveError = null
                        allowPlaintext = false
                        Snackbars.show(if (saved.security == "none") "Share-Server entfernt – nur LAN" else "Share-Server gespeichert")
                    } catch (e: CoreException) {
                        saveError = "Nicht gespeichert: ${e.message ?: e.kind}"
                    } finally {
                        saving = false
                    }
                }
            },
            enabled = info != null && loadError == null && !loading && !saving && (!insecureInput || allowPlaintext),
        ) { Text(if (saving) "Speichere …" else if (saveError != null) "Erneut speichern" else "Speichern") }
    }
    HintLine("Leer lassen = kein Server; Geräte finden sich dann nur im selben WLAN.")
}

/** Only an input hint: native parsing validates endpoints, certificates and mixed transports. */
internal fun usesPlaintext(value: String): Boolean = value.split(',', ';').any { endpoint ->
    endpoint.trim().substringBefore("://", "").lowercase() in setOf("tcp", "ws", "http")
}
