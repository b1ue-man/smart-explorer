package app.smartexplorer.android.ui.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import app.smartexplorer.android.api.DocsApi
import app.smartexplorer.android.api.DocsSettings
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.system.documents.SeDocumentsProvider
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SectionHeader
import app.smartexplorer.android.ui.more.ToggleSetting
import kotlinx.coroutines.launch

/** The limits of the platform, shown under the switches (strand S2 C3 „Grenzen“). */
private const val LIMITS =
    "Andere Apps sehen die freigegebenen Ordner in der Systemauswahl und in „Dateien“. " +
        "Apps, die nur Dateipfade kennen, sehen sie nicht; die Galerie (MediaStore) zeigt sie nicht. " +
        "Geänderte Dateien werden nach dem Schließen hochgeladen; wer mitten in eine Datei schreibt, " +
        "lädt die ganze Datei neu hoch. Offline-Geräte melden „Gerät nicht erreichbar“. " +
        "Tresore erscheinen nur, solange sie entsperrt sind; beim Sperren enden auch offene Dateien."

/**
 * „Ordner für Apps bereitstellen“ (DRV-M8): the main switch turns the documents provider on or
 * off (the component itself, so no other app can even ask while it is off), then single roots:
 * the paired devices, each saved connection, Google Drive and each vault. Nothing is released
 * by default.
 */
@Composable
fun DocumentsProviderSection(modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var settings by remember { mutableStateOf<DocsSettings?>(null) }
    LaunchedEffect(Unit) {
        settings = try {
            DocsApi.settings().also { SeDocumentsProvider.setEnabled(context, it.provider) }
        } catch (e: CoreException) {
            Snackbars.show("Einstellungen nicht lesbar: ${e.message ?: e.kind}")
            null
        }
    }
    fun change(work: suspend () -> DocsSettings) {
        scope.launch {
            try {
                val next = work()
                SeDocumentsProvider.setEnabled(context, next.provider)
                settings = next
            } catch (e: CoreException) {
                Snackbars.show("Nicht gespeichert: ${e.message ?: e.kind}")
            }
        }
    }
    Column(modifier) {
        SectionHeader("Ordner für Apps bereitstellen")
        val current = settings ?: return@Column
        ToggleSetting("Für andere Apps bereitstellen", current.provider, { on ->
            change { DocsApi.setSettings(provider = on) }
        })
        ToggleSetting("Verbundene Geräte", current.devices, { on ->
            change { DocsApi.setSettings(devices = on) }
        }, enabled = current.provider)
        current.choices.connections.forEach { choice ->
            ToggleSetting(choice.label, choice.id in current.connections, { on ->
                val next = if (on) current.connections + choice.id else current.connections - choice.id
                change { DocsApi.setSettings(connections = next) }
            }, enabled = current.provider)
        }
        current.choices.vaults.forEach { choice ->
            val state = if (choice.unlocked) "entsperrt" else "gesperrt"
            ToggleSetting("${choice.label} (Tresor, $state)", choice.id in current.vaults, { on ->
                val next = if (on) current.vaults + choice.id else current.vaults - choice.id
                change { DocsApi.setSettings(vaults = next) }
            }, enabled = current.provider)
        }
        HintLine(LIMITS)
    }
}
