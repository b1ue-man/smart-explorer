package app.smartexplorer.android.ui.viewer

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import app.smartexplorer.android.R
import app.smartexplorer.android.api.LanguageInfoDto
import app.smartexplorer.android.api.TextApi
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.system.Opener
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.theme.isAppInDarkTheme
import kotlinx.coroutines.launch

/**
 * Full-screen text view over [stack] (plan docs/plaene/2026-11-27-dateien-suche-ansicht, D/E): the
 * top entry is shown; a relative Markdown link pushes the linked file ([onPush]), Back pops the top
 * entry ([onPop]), so the view closes after the first one.
 */
@Composable
internal fun TextViewerHost(stack: List<TextViewRequest>, onPush: (TextViewRequest) -> Unit, onPop: () -> Unit) {
    val top = stack.lastOrNull() ?: return
    Dialog(onDismissRequest = onPop, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            // One state per stack entry; a new key starts a new document.
            androidx.compose.runtime.key(stack.size, top) {
                TextViewerScreen(top, onOpenFile = onPush, onClose = onPop)
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TextViewerScreen(request: TextViewRequest, onOpenFile: (TextViewRequest) -> Unit, onClose: () -> Unit) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val state = remember(request) { TextViewerState(scope, request) }
    val dark = isAppInDarkTheme()
    var finding by remember { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    var goTo by remember { mutableStateOf(false) }
    var choosing by remember { mutableStateOf(false) }
    val jump = remember { mutableStateOf<Long?>(if (request.line > 0) request.line else null) }
    LaunchedEffect(state) { state.start(dark) }
    LaunchedEffect(dark) { state.setTheme(dark) }
    DisposableEffect(state) { onDispose { state.close() } }
    BackHandler(onBack = onClose)
    val opened = state.opened
    val isMarkdown = opened?.markdown == true
    val showRendered = isMarkdown && state.rendered

    Column(Modifier.fillMaxSize().safeDrawingPadding().imePadding()) {
        TopAppBar(
            title = {
                Column {
                    Text(request.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(
                        subtitle(state),
                        style = MaterialTheme.typography.bodySmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            },
            navigationIcon = { IconButton(onClick = onClose) { SeIcon(R.drawable.ic_arrow_back, contentDescription = "Zurück") } },
            actions = {
                if (!showRendered) {
                    IconButton(onClick = { finding = !finding }) {
                        SeIcon(if (finding) R.drawable.ic_close else R.drawable.ic_search, contentDescription = if (finding) "Suche schließen" else "Suchen")
                    }
                }
                Box {
                    IconButton(onClick = { menu = true }) { SeIcon(R.drawable.ic_more_vert, contentDescription = "Menü") }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        if (isMarkdown) {
                            DropdownMenuItem(
                                text = { Text(if (state.rendered) "Quelltext zeigen" else "Darstellung zeigen") },
                                onClick = { menu = false; state.rendered = !state.rendered },
                            )
                        }
                        DropdownMenuItem(text = { Text("Sprache…") }, enabled = !showRendered, onClick = { menu = false; choosing = true })
                        DropdownMenuItem(
                            text = { Text(if (state.wrap) "Zeilenumbruch aus" else "Zeilenumbruch an") },
                            enabled = !showRendered,
                            onClick = { menu = false; state.wrap = !state.wrap },
                        )
                        DropdownMenuItem(text = { Text("Gehe zu Zeile…") }, enabled = !showRendered, onClick = { menu = false; goTo = true })
                        DropdownMenuItem(text = { Text("Alles kopieren") }, onClick = {
                            menu = false
                            scope.launch { copyAll(context, state) }
                        })
                        DropdownMenuItem(text = { Text("Pfad kopieren") }, onClick = {
                            menu = false
                            if (Opener.copyText(context, "Pfad", request.location)) Snackbars.show("Pfad kopiert")
                        })
                    }
                }
            },
        )
        LoadingBar(opened == null && state.error == null || state.status?.complete == false)
        if (finding && !showRendered) FindBar(state, onFound = { jump.value = it })
        val error = state.error
        when {
            opened == null && error != null -> ErrorCard(error, modifier = Modifier.padding(16.dp), title = "Ansicht nicht möglich")
            opened == null -> Unit
            showRendered -> {
                LaunchedEffect(opened.docId, state.dark) { state.loadMarkdown() }
                val document = state.markdown
                if (document == null) {
                    state.markdownError?.let { Text(it, modifier = Modifier.padding(16.dp)) }
                } else {
                    MarkdownView(
                        document,
                        MdContext(opened.prefix, opened.path, state.colors) { target ->
                            when (val link = resolveLink(opened.prefix, opened.path, target)) {
                                is LinkTarget.External -> openUrl(context, link.url)
                                is LinkTarget.File -> onOpenFile(TextViewRequest(link.location, link.name))
                                LinkTarget.None -> Unit
                            }
                        },
                    )
                }
            }
            else -> {
                if (error != null) Text(error, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(horizontal = 16.dp))
                TextLinesView(state, jump)
            }
        }
    }
    if (goTo) GoToDialog(state, onDismiss = { goTo = false }) { index ->
        goTo = false
        jump.value = index
    }
    if (choosing) LanguageDialog(state, onDismiss = { choosing = false })
}

private fun subtitle(state: TextViewerState): String {
    val status = state.status
    val parts = buildList {
        if (state.shownLanguage.isNotEmpty()) add(state.shownLanguage)
        (status?.encoding ?: state.opened?.encoding)?.takeIf { it.isNotEmpty() }?.let { add(it) }
        status?.let { known -> known.lineEnding?.let { add(if (known.mixedEndings) "$it (gemischt)" else it) } }
        add(if (status?.complete == true) "${state.known} Zeilen" else "${state.known} Zeilen…")
    }
    return parts.joinToString(" · ")
}

@Composable
private fun FindBar(state: TextViewerState, onFound: (Long) -> Unit) {
    val scope = rememberCoroutineScope()
    var pattern by remember { mutableStateOf("") }
    var regex by remember { mutableStateOf(false) }
    var matchCase by remember { mutableStateOf(false) }
    var note by remember { mutableStateOf("") }
    fun run(backward: Boolean) {
        if (pattern.isEmpty()) return
        scope.launch {
            val found = state.find(pattern, regex, matchCase, backward, fromTop = 0)
            note = when {
                found == null -> "Nicht gefunden"
                found.wrapped -> "Zeile ${found.line + 1} (vom anderen Ende weiter)"
                else -> "Zeile ${found.line + 1}"
            }
            found?.let { onFound(it.line) }
        }
    }
    Column(Modifier.fillMaxWidth().padding(horizontal = 12.dp)) {
        OutlinedTextField(
            value = pattern,
            onValueChange = {
                pattern = it
                state.clearMatch()
            },
            placeholder = { Text("Suchen in der Datei") },
            singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions = KeyboardActions(onSearch = { run(backward = false) }),
            modifier = Modifier.fillMaxWidth(),
        )
        Row(verticalAlignment = Alignment.CenterVertically) {
            Checkbox(checked = regex, onCheckedChange = { regex = it; state.clearMatch() })
            Text("Regex")
            Checkbox(checked = matchCase, onCheckedChange = { matchCase = it; state.clearMatch() })
            Text("Groß/Klein")
            Text(note, style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f).padding(start = 8.dp), maxLines = 1)
            TextButton(onClick = { run(backward = true) }) { Text("Zurück") }
            TextButton(onClick = { run(backward = false) }) { Text("Weiter") }
        }
    }
}

@Composable
private fun GoToDialog(state: TextViewerState, onDismiss: () -> Unit, onGo: (Long) -> Unit) {
    var input by remember { mutableStateOf("") }
    var problem by remember { mutableStateOf<String?>(null) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Gehe zu Zeile") },
        text = {
            Column {
                OutlinedTextField(
                    value = input,
                    onValueChange = { input = it.filter(Char::isDigit); problem = null },
                    label = { Text("Zeile (1 bis ${state.known})") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                )
                problem?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = {
            TextButton(onClick = {
                when (val target = resolveGoToLine(input, state.known, state.status?.complete == true)) {
                    is GoToLine.Line -> onGo(target.index)
                    is GoToLine.Invalid -> problem = target.message
                }
            }) { Text("Gehen") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } },
    )
}

@Composable
private fun LanguageDialog(state: TextViewerState, onDismiss: () -> Unit) {
    val languages by produceState<List<LanguageInfoDto>?>(null) {
        value = try {
            TextApi.languages()
        } catch (e: CoreException) {
            emptyList()
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Sprache") },
        text = {
            val list = languages
            if (list == null) {
                LoadingBar(true)
            } else {
                LazyColumn {
                    item {
                        LanguageRow("Automatisch (${state.opened?.language.orEmpty()})", state.language == null) {
                            state.chooseLanguage(null)
                            onDismiss()
                        }
                    }
                    items(list, key = { it.name }) { language ->
                        LanguageRow(language.name, state.language == language.name) {
                            state.chooseLanguage(language.name)
                            onDismiss()
                        }
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Schließen") } },
    )
}

@Composable
private fun LanguageRow(label: String, selected: Boolean, onClick: () -> Unit) {
    TextButton(onClick = onClick, modifier = Modifier.fillMaxWidth()) {
        Text(
            if (selected) "✓ $label" else label,
            modifier = Modifier
                .fillMaxWidth()
                .background(if (selected) MaterialTheme.colorScheme.secondaryContainer else androidx.compose.ui.graphics.Color.Transparent),
        )
    }
}

/** Largest text "Alles kopieren" puts on the clipboard (the Binder transaction holds about 1 MB). */
private const val MAX_COPY_CHARS = 256 * 1024

private suspend fun copyAll(context: Context, state: TextViewerState) {
    val id = state.opened?.docId ?: return
    val out = StringBuilder()
    var first = 0L
    var count = 0L
    try {
        while (true) {
            // The document's own language: no second highlighting session is built for copying.
            val answer = TextApi.lines(id, first, 1000, state.dark, state.language)
            answer.lines.forEach { line ->
                if (count++ > 0) out.append('\n')
                out.append(line.text)
            }
            if (out.length > MAX_COPY_CHARS) {
                Snackbars.show("Zu groß für die Zwischenablage – bitte einen Ausschnitt markieren und kopieren.")
                return
            }
            if (answer.lines.size < 1000) break
            first += answer.lines.size
        }
    } catch (e: CoreException) {
        Snackbars.show("Kopieren fehlgeschlagen: ${e.message ?: e.kind}")
        return
    }
    if (Opener.copyText(context, state.request.name, out.toString())) Snackbars.show("Text kopiert")
}

private fun openUrl(context: Context, url: String) {
    try {
        context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    } catch (e: ActivityNotFoundException) {
        Snackbars.show("Keine App für diesen Link")
    }
}
