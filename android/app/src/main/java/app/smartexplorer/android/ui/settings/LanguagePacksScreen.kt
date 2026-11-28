package app.smartexplorer.android.ui.settings

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Base64
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.LanguagePackDto
import app.smartexplorer.android.api.TextApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.SubPageScaffold
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Largest syntax file read from the picker (the core refuses more, api.md §4.16). */
private const val MAX_PACK_BYTES = 16 * 1024 * 1024

/**
 * Settings page "Sprachpakete" (api.md §4.16): the languages of the text view, added packs first.
 * [Hinzufügen] takes a `.sublime-syntax` file from the system file picker; an added pack can be
 * removed, a built-in language not (an added pack of the same name or extension takes over).
 */
@Composable
internal fun LanguagePacksScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var packs by remember { mutableStateOf<List<LanguagePackDto>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var reload by remember { mutableIntStateOf(0) }
    var removing by remember { mutableStateOf<LanguagePackDto?>(null) }
    var busy by remember { mutableStateOf(false) }
    LaunchedEffect(reload) {
        try {
            packs = TextApi.packs()
            error = null
        } catch (e: CoreException) {
            error = e.message ?: e.kind
        }
    }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        busy = true
        scope.launch {
            try {
                val (name, bytes) = withContext(Dispatchers.IO) { readPick(context, uri) }
                val added = TextApi.addPack(name, Base64.encodeToString(bytes, Base64.NO_WRAP))
                Snackbars.show("Sprachpaket „${added.name}“ hinzugefügt")
                reload++
            } catch (e: CoreException) {
                Snackbars.show("Hinzufügen fehlgeschlagen: ${e.message ?: e.kind}")
            } catch (e: IOException) {
                Snackbars.show("Datei nicht lesbar: ${e.message ?: "unbekannter Fehler"}")
            } catch (e: SecurityException) {
                Snackbars.show("Kein Zugriff auf die Datei: ${e.message ?: "verweigert"}")
            } finally {
                busy = false
            }
        }
    }
    SubPageScaffold(title = "Sprachpakete", onBack = onBack) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            LoadingBar(packs == null && error == null || busy)
            Text(
                "Die Textansicht färbt Quelltext mit Sublime-Syntax-Dateien (.sublime-syntax). Ein hinzugefügtes Paket " +
                    "geht einer eingebauten Sprache mit gleichem Namen oder gleicher Endung vor.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            )
            OutlinedButton(
                onClick = { picker.launch(arrayOf("*/*")) },
                enabled = !busy,
                modifier = Modifier.padding(horizontal = 16.dp),
            ) { Text("Sprachpaket hinzufügen…") }
            error?.let { ErrorCard(it, modifier = Modifier.padding(16.dp), title = "Sprachpakete nicht geladen", actionLabel = "Erneut", onAction = { reload++ }) }
            LazyColumn(Modifier.fillMaxSize()) {
                items(packs.orEmpty(), key = { "${it.origin}:${it.name}" }) { pack ->
                    ListItem(
                        headlineContent = { Text(pack.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        supportingContent = { Text(describe(pack), maxLines = 3, overflow = TextOverflow.Ellipsis) },
                        trailingContent = if (pack.origin == "added") {
                            {
                                IconButton(onClick = { removing = pack }, enabled = !busy) {
                                    SeIcon(R.drawable.ic_delete, contentDescription = "${pack.name} entfernen")
                                }
                            }
                        } else {
                            null
                        },
                    )
                }
            }
        }
    }
    removing?.let { pack ->
        AlertDialog(
            onDismissRequest = { removing = null },
            title = { Text("Sprachpaket entfernen?") },
            text = { Text("„${pack.name}“ wird aus dem Sprachordner gelöscht.") },
            confirmButton = {
                TextButton(onClick = {
                    removing = null
                    scope.launch {
                        try {
                            TextApi.removePack(pack.name)
                            Snackbars.show("Sprachpaket „${pack.name}“ entfernt")
                        } catch (e: CoreException) {
                            Snackbars.show("Entfernen fehlgeschlagen: ${e.message ?: e.kind}")
                        }
                        reload++
                    }
                }) { Text("Entfernen") }
            },
            dismissButton = { TextButton(onClick = { removing = null }) { Text("Abbrechen") } },
        )
    }
}

private fun describe(pack: LanguagePackDto): String = buildList {
    add(if (pack.origin == "added") "Hinzugefügt" else "Eingebaut")
    pack.shadowedBy?.let { add("ersetzt durch „$it“") }
    val extensions = pack.languages.flatMap { it.extensions }.distinct()
    if (extensions.isNotEmpty()) add(extensions.take(8).joinToString(", ") + if (extensions.size > 8) ", …" else "")
    if (pack.problems.isNotEmpty()) add("Probleme: " + pack.problems.joinToString("; "))
}.joinToString(" · ")

/** Display name and bytes of the picked file; more than [MAX_PACK_BYTES] is refused. */
private fun readPick(context: Context, uri: Uri): Pair<String, ByteArray> {
    val resolver = context.contentResolver
    val name = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
        if (cursor.moveToFirst()) cursor.getString(0) else null
    } ?: uri.lastPathSegment?.substringAfterLast('/') ?: "sprache.sublime-syntax"
    val bytes = resolver.openInputStream(uri)?.use { input ->
        val out = java.io.ByteArrayOutputStream()
        val buffer = ByteArray(64 * 1024)
        while (true) {
            val read = input.read(buffer)
            if (read < 0) break
            out.write(buffer, 0, read)
            if (out.size() > MAX_PACK_BYTES) throw IOException("größer als 16 MiB")
        }
        out.toByteArray()
    } ?: throw IOException("Datei lässt sich nicht öffnen")
    return name to bytes
}
