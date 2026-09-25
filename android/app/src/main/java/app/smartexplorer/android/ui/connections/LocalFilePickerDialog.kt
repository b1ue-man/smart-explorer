// material3 1.4.0: TopAppBar may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.connections

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.R
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.Listing
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.core.Root
import app.smartexplorer.android.core.SortSpec
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.ui.common.EmptyState
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.kindIcon
import app.smartexplorer.android.ui.files.CrumbBar

/**
 * Full-screen picker for local files (SSH key file, files to send to a Share device). Only the
 * storage volumes are offered. [multiple] = check boxes and [Auswählen]; otherwise a tap picks.
 * Back goes to the previously shown folder, then closes via [onDismiss].
 */
@Composable
internal fun LocalFilePickerDialog(
    title: String,
    multiple: Boolean,
    onPick: (List<String>) -> Unit,
    onDismiss: () -> Unit,
    showHiddenInitially: Boolean = false,
) {
    val dirsFirst by AppPrefs.dirsFirst.collectAsStateWithLifecycle()
    var showHidden by remember { mutableStateOf(showHiddenInitially) }
    var volumes by remember { mutableStateOf<List<Root>>(emptyList()) }
    var location by remember { mutableStateOf<String?>(null) }
    val history = remember { mutableStateListOf<String>() }
    var listing by remember { mutableStateOf<Listing?>(null) }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var reloadKey by remember { mutableIntStateOf(0) }
    val selected = remember { mutableStateListOf<String>() }

    LaunchedEffect(Unit) {
        try {
            volumes = FilesApi.roots().storage
            if (location == null) location = volumes.firstOrNull()?.location
            if (volumes.isEmpty()) error = "Kein Speicher verfügbar"
        } catch (e: CoreException) {
            error = e.message ?: e.kind
        }
    }
    LaunchedEffect(location, showHidden, dirsFirst, reloadKey) {
        val target = location ?: return@LaunchedEffect
        loading = true
        try {
            listing = FilesApi.list(target, showHidden, filter = null, sort = SortSpec(key = "name", dirsFirst = dirsFirst))
            error = null
        } catch (e: CoreException) {
            error = e.message ?: e.kind
        } finally {
            if (target == location) loading = false
        }
    }

    fun go(next: String) {
        location?.let { if (it != next) history.add(it) }
        location = next
    }

    Dialog(
        onDismissRequest = { if (history.isNotEmpty()) location = history.removeAt(history.lastIndex) else onDismiss() },
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surface) {
            Column(Modifier.fillMaxSize()) {
                TopAppBar(
                    title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    navigationIcon = {
                        IconButton(onClick = onDismiss) { SeIcon(R.drawable.ic_close, contentDescription = "Abbrechen") }
                    },
                    actions = {
                        IconButton(onClick = { showHidden = !showHidden }) {
                            SeIcon(
                                R.drawable.ic_visibility,
                                contentDescription = if (showHidden) "Versteckte ausblenden" else "Versteckte zeigen",
                                tint = if (showHidden) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    },
                )
                if (volumes.size > 1) {
                    Row(
                        Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        volumes.forEach { volume ->
                            FilterChip(
                                selected = location?.startsWith(volume.location) == true,
                                onClick = { go(volume.location) },
                                label = { Text(volume.label) },
                            )
                        }
                    }
                }
                listing?.let { CrumbBar(it.crumbs, onOpen = { target -> go(target) }) }
                LoadingBar(loading)
                val current = listing
                val message = error
                Column(Modifier.weight(1f).fillMaxWidth()) {
                    when {
                        message != null -> ErrorCard(
                            message,
                            modifier = Modifier.padding(16.dp),
                            title = "Ordner nicht lesbar",
                            actionLabel = "Erneut",
                            onAction = { reloadKey++ },
                        )
                        current == null -> Unit
                        current.entries.isEmpty() -> EmptyState(R.drawable.ic_folder, "Dieser Ordner ist leer")
                        else -> LazyColumn(Modifier.fillMaxSize()) {
                            items(current.entries, key = { it.location }) { entry ->
                                PickerRow(
                                    entry,
                                    multiple = multiple,
                                    checked = entry.location in selected,
                                    onClick = {
                                        when {
                                            entry.isDir -> go(entry.location)
                                            !multiple -> onPick(listOf(entry.location))
                                            entry.location in selected -> selected.remove(entry.location)
                                            else -> selected.add(entry.location)
                                        }
                                    },
                                )
                            }
                        }
                    }
                }
                if (multiple) {
                    HorizontalDivider()
                    Button(
                        onClick = { onPick(selected.toList()) },
                        enabled = selected.isNotEmpty(),
                        modifier = Modifier.fillMaxWidth().navigationBarsPadding().padding(16.dp),
                    ) { Text(if (selected.isEmpty()) "Dateien antippen" else "Auswählen (${selected.size})") }
                }
            }
        }
    }
}

@Composable
private fun PickerRow(entry: Entry, multiple: Boolean, checked: Boolean, onClick: () -> Unit) {
    ListItem(
        headlineContent = { Text(entry.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = if (entry.isDir) {
            null
        } else {
            { Text("${Format.size(entry.size)} · ${Format.date(entry.mtimeMs)}") }
        },
        leadingContent = { SeIcon(kindIcon(entry.kind), contentDescription = null) },
        trailingContent = if (!entry.isDir && multiple) {
            { Checkbox(checked = checked, onCheckedChange = null) }
        } else {
            null
        },
        modifier = Modifier.clickable(onClick = onClick),
    )
}
