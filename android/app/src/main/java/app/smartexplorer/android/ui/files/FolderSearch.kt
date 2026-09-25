package app.smartexplorer.android.ui.files

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.R
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.FolderHit
import app.smartexplorer.android.api.IndexStatus
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.EmptyState
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Folder search over the folder index of the storage volumes (spec F6). */
internal class FolderSearchState(private val scope: CoroutineScope) {
    var query by mutableStateOf("")
        private set
    var hits by mutableStateOf<List<FolderHit>>(emptyList())
        private set
    var searching by mutableStateOf(false)
        private set
    var status by mutableStateOf<IndexStatus?>(null)
        private set

    /** Index build started from this page (progress, [Index neu aufbauen]). */
    var buildTaskId by mutableStateOf<String?>(null)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    private var searchJob: Job? = null

    /** Page opened: build the index on first use; searching already uses the built part. */
    fun opened() {
        scope.launch {
            try {
                val current = FilesApi.indexStatus()
                status = current
                if (current.state == "none") build()
            } catch (e: CoreException) {
                error = e.message ?: e.kind
            }
        }
        if (query.isNotBlank()) search(immediate = true)
    }

    fun onQuery(text: String) {
        query = text
        search(immediate = false)
    }

    /** [Index neu aufbauen] */
    fun rebuild() {
        scope.launch { build() }
    }

    private suspend fun build() {
        if (buildTaskId != null) return
        try {
            val id = FilesApi.indexBuild()
            buildTaskId = id
            val task = FilesApi.awaitTask(id)
            if (task.state == "failed") error = task.message ?: "Index konnte nicht aufgebaut werden"
            status = FilesApi.indexStatus()
        } catch (e: CoreException) {
            error = e.message ?: e.kind
        } finally {
            buildTaskId = null
        }
        search(immediate = true)
    }

    private fun search(immediate: Boolean) {
        searchJob?.cancel()
        val text = query.trim()
        if (text.isEmpty()) {
            hits = emptyList()
            searching = false
            return
        }
        searchJob = scope.launch {
            if (!immediate) delay(SEARCH_DEBOUNCE_MS)
            searching = true
            try {
                hits = FilesApi.indexSearch(text, MAX_HITS).distinctBy { it.location }
                error = null
            } catch (e: CoreException) {
                error = e.message ?: e.kind
            } finally {
                searching = false
            }
        }
    }

    private companion object {
        const val SEARCH_DEBOUNCE_MS = 250L
        const val MAX_HITS = 200
    }
}

/** Search page: field, hits (folder name, path); tap opens the folder in the current tab. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun FolderSearchPage(state: FolderSearchState, onOpen: (String) -> Unit, onBack: () -> Unit) {
    val tasks by Core.tasks.collectAsStateWithLifecycle()
    val focus = remember { FocusRequester() }
    var menu by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { state.opened() }
    BackHandler(onBack = onBack)
    val buildTask = tasks.firstOrNull { it.id == state.buildTaskId }
        ?: tasks.firstOrNull { it.kind == "index" && it.isActive }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Ordner suchen") },
                navigationIcon = {
                    IconButton(onClick = onBack) { SeIcon(R.drawable.ic_arrow_back, contentDescription = "Zurück") }
                },
                actions = {
                    Box {
                        IconButton(onClick = { menu = true }) { SeIcon(R.drawable.ic_more_vert, contentDescription = "Menü") }
                        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                            DropdownMenuItem(
                                text = { Text("Index neu aufbauen") },
                                enabled = buildTask == null,
                                onClick = {
                                    menu = false
                                    state.rebuild()
                                },
                            )
                        }
                    }
                },
            )
        },
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
    ) { padding ->
        // Same (sub)composition as the field, so the requester is attached when this runs.
        LaunchedEffect(Unit) { focus.requestFocus() }
        Column(Modifier.fillMaxSize().padding(padding)) {
            OutlinedTextField(
                value = state.query,
                onValueChange = { state.onQuery(it) },
                placeholder = { Text("Ordnername oder Teil davon") },
                leadingIcon = { SeIcon(R.drawable.ic_search, contentDescription = null) },
                trailingIcon = if (state.query.isNotEmpty()) {
                    { IconButton(onClick = { state.onQuery("") }) { SeIcon(R.drawable.ic_close, contentDescription = "Leeren") } }
                } else {
                    null
                },
                singleLine = true,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).focusRequester(focus),
            )
            LoadingBar(state.searching || buildTask != null)
            if (buildTask != null) {
                Text(
                    "${buildTask.doneItems} Ordner indiziert – die Suche nutzt bereits gebaute Teile.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                )
            }
            val message = state.error
            when {
                message != null && state.hits.isEmpty() -> ErrorCard(
                    message,
                    modifier = Modifier.padding(16.dp),
                    title = "Ordnersuche nicht verfügbar",
                    actionLabel = "Index neu aufbauen",
                    onAction = { state.rebuild() },
                )
                state.query.isBlank() -> EmptyState(
                    R.drawable.ic_search,
                    "Ordner per Namensteil finden",
                    message = state.status?.takeIf { it.state == "ready" }?.let { "${it.count} Ordner im Index" },
                )
                state.hits.isEmpty() && !state.searching -> EmptyState(
                    R.drawable.ic_folder,
                    "Kein Ordner gefunden",
                    message = if (buildTask != null) "Der Index wird noch aufgebaut." else null,
                )
                else -> LazyColumn(Modifier.fillMaxSize()) {
                    items(state.hits, key = { it.location }) { hit ->
                        ListItem(
                            headlineContent = { Text(hit.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                            supportingContent = { Text(hit.path, maxLines = 2, overflow = TextOverflow.Ellipsis) },
                            leadingContent = { SeIcon(R.drawable.ic_folder, contentDescription = null) },
                            modifier = Modifier.clickable { onOpen(hit.location) },
                        )
                    }
                }
            }
        }
    }
}
