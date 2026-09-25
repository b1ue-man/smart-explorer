package app.smartexplorer.android.ui.picker

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
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
import androidx.compose.runtime.rememberCoroutineScope
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
import app.smartexplorer.android.api.Roots
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.core.Root
import app.smartexplorer.android.core.SortSpec
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.ui.common.EmptyState
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.common.rootIcon
import app.smartexplorer.android.ui.files.CrumbBar
import app.smartexplorer.android.ui.files.NameDialog
import app.smartexplorer.android.ui.files.NameDialogKind
import kotlinx.coroutines.launch

/** Backends that only exist inside the app (api.md §2) and never become a target elsewhere. */
private val APP_INTERNAL_BACKENDS = setOf("zip", "trash")

/**
 * Full-screen target picker ("Kopieren nach…", sync sides, receiving shares; spec F7): places,
 * path bar, folder list, [Neuer Ordner], [confirmLabel]. [onPick] gets the location as reported
 * by the core. App-internal places (ZIP contents, trash) are refused unless [allowAppInternal].
 * Back goes to the previously shown folder, then closes via [onDismiss].
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LocationPickerDialog(
    title: String,
    initialLocation: String?,
    confirmLabel: String,
    allowAppInternal: Boolean = false,
    onPick: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val showHidden by AppPrefs.showHidden.collectAsStateWithLifecycle()
    val dirsFirst by AppPrefs.dirsFirst.collectAsStateWithLifecycle()
    var location by remember { mutableStateOf(initialLocation) }
    val history = remember { mutableStateListOf<String>() }
    var showPlaces by remember { mutableStateOf(initialLocation == null) }
    var roots by remember { mutableStateOf<Roots?>(null) }
    var listing by remember { mutableStateOf<Listing?>(null) }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var rootsError by remember { mutableStateOf<String?>(null) }
    var reloadKey by remember { mutableIntStateOf(0) }
    var rootsKey by remember { mutableIntStateOf(0) }
    var creating by remember { mutableStateOf(false) }

    LaunchedEffect(rootsKey) {
        try {
            val loaded = FilesApi.roots()
            roots = loaded
            rootsError = null
            if (location == null) location = loaded.storage.firstOrNull()?.location
        } catch (e: CoreException) {
            rootsError = e.message ?: e.kind
        }
    }
    LaunchedEffect(location, showHidden, dirsFirst, reloadKey) {
        val target = location ?: return@LaunchedEffect
        loading = true
        try {
            val result = FilesApi.list(target, showHidden, filter = null, sort = SortSpec(key = "name", dirsFirst = dirsFirst))
            listing = result
            error = null
        } catch (e: CoreException) {
            error = e.message ?: e.kind
        } finally {
            // A newer place may already be loading; only the current one ends the indicator.
            if (target == location) loading = false
        }
    }

    fun go(next: String) {
        val current = listing?.location ?: location
        if (current != null && current != next) history.add(current)
        location = next
        showPlaces = false
    }

    fun back() {
        when {
            showPlaces && location != null -> showPlaces = false
            history.isNotEmpty() -> location = history.removeAt(history.lastIndex)
            else -> onDismiss()
        }
    }

    val current = listing
    val internal = current != null && current.backend in APP_INTERNAL_BACKENDS
    val canConfirm = current != null && !loading && error == null && !showPlaces && (allowAppInternal || !internal)

    Dialog(
        // Back (and only back: the dialog fills the screen) walks the picker history first.
        onDismissRequest = { back() },
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
                        IconButton(onClick = { showPlaces = !showPlaces }) {
                            SeIcon(R.drawable.ic_storage, contentDescription = "Orte")
                        }
                    },
                )
                if (!showPlaces && current != null) {
                    CrumbBar(current.crumbs, onOpen = { go(it) })
                }
                LoadingBar(loading)
                Column(Modifier.weight(1f).fillMaxWidth()) {
                    val message = error
                    when {
                        showPlaces -> PlacesList(roots, rootsError, allowAppInternal, onOpen = { go(it) }, onRetry = { rootsKey++ })
                        message != null -> ErrorCard(
                            message,
                            modifier = Modifier.padding(16.dp),
                            title = "Ort nicht erreichbar",
                            actionLabel = "Erneut",
                            onAction = { reloadKey++ },
                        )
                        current != null -> FolderList(current.entries.filter { it.isDir }, onOpen = { go(it.location) })
                    }
                }
                if (internal && !allowAppInternal && !showPlaces) {
                    Text(
                        "Dieser Ort ist nur in der App sichtbar und kann hier nicht gewählt werden.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                    )
                }
                HorizontalDivider()
                Row(
                    modifier = Modifier.fillMaxWidth().navigationBarsPadding().padding(16.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    OutlinedButton(
                        onClick = { creating = true },
                        enabled = canConfirm && current?.readOnly == false,
                        modifier = Modifier.weight(1f),
                    ) { Text("Neuer Ordner") }
                    Button(
                        onClick = { current?.let { onPick(it.location) } },
                        enabled = canConfirm,
                        modifier = Modifier.weight(1f),
                    ) { Text(confirmLabel, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                }
            }
        }
    }

    val parent = current?.location
    if (creating && parent != null) {
        NameDialog(
            kind = NameDialogKind.NewFolder,
            parent = parent,
            initialName = "Neuer Ordner",
            onDismiss = { creating = false },
            onConfirm = { name ->
                creating = false
                scope.launch {
                    try {
                        val created = FilesApi.mkdir(parent, name)
                        go(created.location)
                    } catch (e: CoreException) {
                        Snackbars.show(e.message ?: e.kind)
                    }
                }
            },
        )
    }
}

@Composable
private fun PlacesList(roots: Roots?, error: String?, allowAppInternal: Boolean, onOpen: (String) -> Unit, onRetry: () -> Unit) {
    if (error != null) {
        ErrorCard(error, modifier = Modifier.padding(16.dp), title = "Orte nicht geladen", actionLabel = "Erneut", onAction = onRetry)
        return
    }
    if (roots == null) return
    val sections = listOf(
        "Speicher" to roots.storage,
        "Favoriten" to roots.favorites,
        "Zuletzt" to roots.recent.take(10),
        "Verbindungen" to roots.connections,
        "Google Drive" to listOfNotNull(roots.gdrive),
        "Share" to roots.devices + roots.rooms,
    ) + if (allowAppInternal) listOf("Papierkorb" to listOfNotNull(roots.trash)) else emptyList()
    LazyColumn(Modifier.fillMaxSize()) {
        sections.filter { it.second.isNotEmpty() }.forEach { (header, list) ->
            item(key = "h:$header") {
                Text(
                    header,
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
                )
            }
            items(list, key = { "$header:${it.id}" }) { root -> PlaceRow(root, onOpen) }
        }
    }
}

@Composable
private fun PlaceRow(root: Root, onOpen: (String) -> Unit) {
    ListItem(
        headlineContent = { Text(root.label, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = if (root.subtitle != null) {
            { Text(root.subtitle, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        } else {
            null
        },
        leadingContent = { SeIcon(rootIcon(root), contentDescription = null) },
        modifier = Modifier.clickable { onOpen(root.location) },
    )
}

@Composable
private fun FolderList(folders: List<Entry>, onOpen: (Entry) -> Unit) {
    if (folders.isEmpty()) {
        EmptyState(R.drawable.ic_folder, "Keine Unterordner", message = "Hier einfügen oder einen neuen Ordner anlegen.")
        return
    }
    LazyColumn(Modifier.fillMaxSize()) {
        items(folders, key = { it.location }) { folder ->
            ListItem(
                headlineContent = { Text(folder.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                leadingContent = { SeIcon(R.drawable.ic_folder, contentDescription = null) },
                trailingContent = { SeIcon(R.drawable.ic_chevron_right, contentDescription = null) },
                modifier = Modifier.clickable { onOpen(folder) },
            )
        }
    }
}
