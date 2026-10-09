// material3 1.4.0: SegmentedButton may still be experimental (docs/refs/compose-material3.md §0, §14).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.settings

import androidx.annotation.DrawableRes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.R
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.system.Permissions
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.common.rememberAllFilesAccess
import app.smartexplorer.android.ui.connections.GoogleDriveSection
import app.smartexplorer.android.ui.more.HintLine
import app.smartexplorer.android.ui.more.SectionHeader
import app.smartexplorer.android.ui.more.SubPageScaffold
import app.smartexplorer.android.ui.more.ToggleSetting
import app.smartexplorer.android.update.UpdateChecker
import app.smartexplorer.android.update.UpdateState

/** Section to scroll to when the settings open (notification targets). */
enum class SettingsFocus { Background, Updates }

/** Sections in display order (spec F21); the index is the lazy-list item index. */
private enum class SettingsItem { Appearance, FileList, Background, ShareServer, Drive, Storage, Updates, ErrorLog, About }

private val THEMES = listOf("system" to "System", "light" to "Hell", "dark" to "Dunkel")

/**
 * "Mehr → Einstellungen" (spec F21): Darstellung · Dateiliste · Hintergrund · Share-Server ·
 * Google Drive · Speicherzugriff · Updates · Fehlerprotokoll · Über. [focus] scrolls to a section
 * once and is then reported via [onFocusHandled].
 */
@Composable
fun SettingsScreen(
    focus: SettingsFocus?,
    onFocusHandled: () -> Unit,
    onBack: () -> Unit,
    onOpenErrorLog: () -> Unit,
    onOpenAbout: () -> Unit,
) {
    val context = LocalContext.current
    val listState = rememberLazyListState()
    LaunchedEffect(focus) {
        val target = when (focus ?: return@LaunchedEffect) {
            SettingsFocus.Background -> SettingsItem.Background
            SettingsFocus.Updates -> {
                // Opened from the update notification after a process restart: show the current state.
                if (UpdateChecker.state.value == UpdateState.Idle) UpdateChecker.checkNow(context)
                SettingsItem.Updates
            }
        }
        listState.animateScrollToItem(target.ordinal)
        onFocusHandled()
    }
    SubPageScaffold(title = "Einstellungen", onBack = onBack) { padding ->
        LazyColumn(Modifier.fillMaxSize().padding(padding).imePadding(), state = listState) {
            SettingsItem.entries.forEach { section ->
                item(key = section.name) {
                    Column {
                        when (section) {
                            SettingsItem.Appearance -> AppearanceSettings()
                            SettingsItem.FileList -> FileListSettings()
                            SettingsItem.Background -> {
                                SectionHeader("Hintergrund")
                                BackgroundSettingsSection(Modifier.padding(horizontal = 16.dp))
                            }
                            SettingsItem.ShareServer -> ShareServerSettings()
                            SettingsItem.Drive -> {
                                SectionHeader("Google Drive")
                                GoogleDriveSection(Modifier.padding(horizontal = 16.dp))
                            }
                            SettingsItem.Storage -> StorageAccessSettings()
                            SettingsItem.Updates -> UpdateSettings()
                            SettingsItem.ErrorLog -> LinkRow("Fehlerprotokoll", "App-Fehler und Absturzprotokoll", R.drawable.ic_warning, onOpenErrorLog)
                            SettingsItem.About -> LinkRow("Über", "Version, Lizenz, Grenzen", R.drawable.ic_info, onOpenAbout)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun AppearanceSettings() {
    val theme by AppPrefs.theme.collectAsStateWithLifecycle()
    SectionHeader("Darstellung")
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        THEMES.forEachIndexed { index, (value, label) ->
            SegmentedButton(
                selected = theme == value,
                onClick = { AppPrefs.setTheme(value) },
                shape = SegmentedButtonDefaults.itemShape(index = index, count = THEMES.size),
                label = { Text(label) },
            )
        }
    }
}

@Composable
private fun FileListSettings() {
    val showHidden by AppPrefs.showHidden.collectAsStateWithLifecycle()
    val dirsFirst by AppPrefs.dirsFirst.collectAsStateWithLifecycle()
    val compact by AppPrefs.compact.collectAsStateWithLifecycle()
    val thumbnails by AppPrefs.thumbnails.collectAsStateWithLifecycle()
    val mediaViewer by AppPrefs.mediaViewer.collectAsStateWithLifecycle()
    SectionHeader("Dateiliste")
    ToggleSetting("Versteckte Dateien zeigen", showHidden, AppPrefs::setShowHidden)
    ToggleSetting("Ordner zuerst", dirsFirst, AppPrefs::setDirsFirst)
    ToggleSetting("Kompakte Zeilen", compact, AppPrefs::setCompact)
    ToggleSetting("Bildvorschau", thumbnails, AppPrefs::setThumbnails)
    ToggleSetting("Medien im eigenen Betrachter öffnen", mediaViewer, AppPrefs::setMediaViewer)
    HintLine(
        if (mediaViewer) {
            "Wischen oder ←/→ blättert durch die Bilder und Videos (bzw. Audiodateien) der Liste."
        } else {
            "Bilder, Videos und Audio öffnen direkt in einer anderen App (ohne Weiterblättern)."
        },
    )
}

@Composable
private fun StorageAccessSettings() {
    val context = LocalContext.current
    val granted = rememberAllFilesAccess()
    SectionHeader("Speicherzugriff")
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            if (granted) "Zugriff auf alle Dateien: erlaubt" else "Zugriff auf alle Dateien: nicht erlaubt",
            style = MaterialTheme.typography.bodyLarge,
            modifier = Modifier.weight(1f),
        )
        if (!granted) {
            OutlinedButton(onClick = {
                if (!Permissions.openAllFilesAccessSettings(context)) {
                    Snackbars.show("Einstellung nicht verfügbar – bitte in den App-Einstellungen erlauben.")
                }
            }) { Text("Erlauben") }
        }
    }
    if (!granted) HintLine("Ohne diesen Zugriff sind nur App-Ordner sichtbar; SD-Karte und USB bleiben verborgen.")
}

@Composable
private fun UpdateSettings() {
    val auto by AppPrefs.autoUpdateCheck.collectAsStateWithLifecycle()
    val last by AppPrefs.lastUpdateCheckMs.collectAsStateWithLifecycle()
    SectionHeader("Updates")
    UpdateCard(Modifier.padding(horizontal = 16.dp))
    ToggleSetting("Beim Start prüfen (höchstens 1× am Tag)", auto, AppPrefs::setAutoUpdateCheck)
    HintLine(if (last > 0) "Zuletzt geprüft: ${Format.dateTime(last)}" else "Noch nicht geprüft")
}

@Composable
private fun LinkRow(title: String, subtitle: String, @DrawableRes icon: Int, onClick: () -> Unit) {
    ListItem(
        headlineContent = { Text(title) },
        supportingContent = { Text(subtitle) },
        leadingContent = { SeIcon(icon, contentDescription = null) },
        trailingContent = { SeIcon(R.drawable.ic_chevron_right, contentDescription = null) },
        modifier = Modifier.clickable(onClick = onClick),
    )
}
