package app.smartexplorer.android.ui.files

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon

private val MODES = listOf(
    "normal" to "Normal",
    "extended" to "Erweitert (\\n, \\r, \\t, \\0, \\x…)",
    "regex" to "Regulärer Ausdruck",
)

/**
 * Page "In Dateien suchen" (plan docs/plaene/2026-11-27-dateien-suche-ansicht, E): the form, the
 * progress with [Abbrechen] and the results grouped by file. Tapping a hit line opens the text
 * view at that line ([onOpenLine]: location, name, zero-based line, UTF-16 range of the first match).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ContentSearchPage(
    state: ContentSearchState,
    onOpenLine: (location: String, name: String, line: Long, start: Int, end: Int) -> Unit,
    onBack: () -> Unit,
) {
    BackHandler(onBack = onBack)
    var showForm by rememberSaveable { mutableStateOf(true) }
    var menu by remember { mutableStateOf(false) }
    val results = state.results
    // Read so the list recomposes after every applied poll answer.
    val version = state.version
    val rows = remember(results, version, state.collapsed) { results?.rows(state.collapsed).orEmpty() }
    val mark = SpanStyle(background = MaterialTheme.colorScheme.tertiaryContainer, fontWeight = FontWeight.Bold)

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("In Dateien suchen") },
                navigationIcon = {
                    IconButton(onClick = onBack) { SeIcon(R.drawable.ic_arrow_back, contentDescription = "Zurück") }
                },
                actions = {
                    IconButton(onClick = { showForm = !showForm }) {
                        SeIcon(
                            if (showForm) R.drawable.ic_expand_less else R.drawable.ic_expand_more,
                            contentDescription = if (showForm) "Suchfelder ausblenden" else "Suchfelder zeigen",
                        )
                    }
                    Box {
                        IconButton(onClick = { menu = true }) { SeIcon(R.drawable.ic_more_vert, contentDescription = "Menü") }
                        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                            DropdownMenuItem(text = { Text("Alle aufklappen") }, enabled = results != null, onClick = {
                                menu = false
                                state.expandAll()
                            })
                            DropdownMenuItem(text = { Text("Alle zuklappen") }, enabled = results != null, onClick = {
                                menu = false
                                state.collapseAll()
                            })
                        }
                    }
                },
            )
        },
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
    ) { padding ->
        LazyColumn(Modifier.fillMaxSize().padding(padding).imePadding()) {
            if (showForm) item(key = "form") { SearchForm(state) }
            item(key = "status") {
                Column {
                    LoadingBar(state.running)
                    state.error?.let { ErrorCard(it, modifier = Modifier.padding(16.dp), title = "Suche nicht möglich") }
                    if (results != null) {
                        // A state read here: this item follows every applied poll answer.
                        val header = remember(results, state.version) { results.header() }
                        Text(
                            header,
                            style = MaterialTheme.typography.titleSmall,
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                        )
                        val notes = buildList {
                            if (state.running) add("Suche läuft…")
                            if (results.canceled) add("Abgebrochen – die bisherigen Treffer bleiben.")
                            if (results.limited) add("Höchstzahl der Treffer erreicht.")
                            results.error?.let { add(it) }
                        }
                        if (notes.isNotEmpty()) {
                            Text(
                                notes.joinToString(" "),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(horizontal = 16.dp),
                            )
                        }
                    }
                }
            }
            items(rows, key = { it.key }) { row ->
                when (row) {
                    is ResultRow.File -> FileHead(row, onToggle = { state.toggle(row.file.rel) }, onOpen = {
                        val hit = row.file.firstHit
                        val range = hit?.ranges?.firstOrNull()
                        onOpenLine(
                            row.file.location,
                            row.file.rel.substringAfterLast('/'),
                            (hit?.line ?: 1L) - 1,
                            range?.getOrNull(0) ?: 0,
                            range?.getOrNull(1) ?: 0,
                        )
                    })
                    is ResultRow.Line -> Text(
                        hitText(row.line, mark),
                        style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                        color = if (row.line.context) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable {
                                val range = row.line.ranges.firstOrNull()
                                onOpenLine(
                                    row.file.location,
                                    row.file.rel.substringAfterLast('/'),
                                    row.line.line - 1,
                                    range?.getOrNull(0) ?: 0,
                                    range?.getOrNull(1) ?: 0,
                                )
                            }
                            .padding(start = 40.dp, end = 16.dp, top = 4.dp, bottom = 4.dp),
                    )
                    is ResultRow.SkippedHeader -> Row(
                        Modifier.fillMaxWidth().clickable { state.toggle(SKIPPED_KEY) }.padding(horizontal = 16.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        SeIcon(if (row.expanded) R.drawable.ic_expand_less else R.drawable.ic_expand_more, contentDescription = null)
                        Text("Ausgelassen (${row.count})", style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(start = 8.dp))
                    }
                    is ResultRow.Skipped -> Text(
                        "${row.skipped.rel}: ${row.skipped.reason}",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.fillMaxWidth().padding(start = 40.dp, end = 16.dp, top = 2.dp, bottom = 2.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun FileHead(row: ResultRow.File, onToggle: () -> Unit, onOpen: () -> Unit) {
    Column {
        HorizontalDivider()
        Row(
            Modifier.fillMaxWidth().clickable(onClick = onToggle).padding(start = 16.dp, top = 6.dp, bottom = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            SeIcon(if (row.expanded) R.drawable.ic_expand_less else R.drawable.ic_expand_more, contentDescription = null)
            Text(
                "${row.file.rel} (${row.file.hits} Treffer)",
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f).padding(start = 8.dp),
            )
            IconButton(onClick = onOpen) { SeIcon(R.drawable.ic_text, contentDescription = "${row.file.rel} ansehen") }
        }
    }
}

@Composable
private fun SearchForm(state: ContentSearchState) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        OutlinedTextField(
            value = state.pattern,
            onValueChange = { state.pattern = it },
            label = { Text("Suchen nach") },
            singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions = KeyboardActions(onSearch = { state.start() }),
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            value = state.filters,
            onValueChange = { state.filters = it },
            label = { Text("Filter") },
            placeholder = { Text("*.kt *.md !*.log !\\build !+\\tmp*") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            value = state.location.orEmpty(),
            onValueChange = {},
            readOnly = true,
            label = { Text("Ordner") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            value = state.folderInclude,
            onValueChange = { state.folderInclude = it },
            label = { Text("Ordnerpfad (Regex)") },
            placeholder = { Text("^src(/|$)") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            value = state.folderExclude,
            onValueChange = { state.folderExclude = it },
            label = { Text("Ordnerpfad ausschließen (Regex)") },
            placeholder = { Text("(^|/)(build|\\.git)$") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        CheckRow("In allen Unterordnern", state.subfolders) { state.subfolders = it }
        CheckRow("In versteckten Ordnern", state.hidden) { state.hidden = it }
        CheckRow("Nur ganze Wörter", state.wholeWord && state.mode != "regex", enabled = state.mode != "regex") { state.wholeWord = it }
        CheckRow("Groß-/Kleinschreibung beachten", state.matchCase) { state.matchCase = it }
        CheckRow("Nur Dateinamen anzeigen", state.filesOnly) { state.filesOnly = it }
        Text("Suchmodus", style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(top = 4.dp))
        MODES.forEach { (value, label) ->
            Row(Modifier.fillMaxWidth().clickable { state.mode = value }, verticalAlignment = Alignment.CenterVertically) {
                RadioButton(selected = state.mode == value, onClick = { state.mode = value })
                Text(label)
            }
        }
        CheckRow(". findet Zeilenumbruch", state.dotMatchesNewline && state.mode == "regex", enabled = state.mode == "regex") {
            state.dotMatchesNewline = it
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                value = state.contextLines,
                onValueChange = { state.contextLines = it.filter(Char::isDigit) },
                label = { Text("Kontextzeilen") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                modifier = Modifier.weight(1f),
            )
            OutlinedTextField(
                value = state.maxHits,
                onValueChange = { state.maxHits = it.filter(Char::isDigit) },
                label = { Text("Höchstzahl Treffer") },
                placeholder = { Text("unbegrenzt") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                modifier = Modifier.weight(1f),
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(top = 4.dp)) {
            Button(onClick = { state.start() }, enabled = state.location != null) { Text("Alle suchen") }
            if (state.running) {
                OutlinedButton(onClick = { state.cancel() }) { Text("Abbrechen") }
            }
            TextButton(onClick = {
                state.pattern = ""
                state.filters = ""
                state.folderInclude = ""
                state.folderExclude = ""
            }) { Text("Leeren") }
        }
    }
}

@Composable
private fun CheckRow(label: String, checked: Boolean, enabled: Boolean = true, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(enabled = enabled) { onChange(!checked) },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = checked, onCheckedChange = onChange, enabled = enabled)
        Text(
            label,
            color = if (enabled) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
