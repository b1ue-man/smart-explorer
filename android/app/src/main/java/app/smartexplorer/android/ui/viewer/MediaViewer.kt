package app.smartexplorer.android.ui.viewer

import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.R
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Entry
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.common.SeIcon
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.launch

/**
 * Full-screen viewer for the media of [set] (plan docs/plaene/2026-10-09-medien-weiterschalten,
 * C3): swipe or ←/→ steps to the previous/next medium in list order; pictures zoom, videos and
 * audio play in place. [onOpenExternal] hands the current medium to another app (the previous
 * direct way; [LocalMedia] when it is already available locally). Back closes.
 */
@Composable
internal fun MediaViewer(
    set: MediaSet,
    onClose: () -> Unit,
    onOpenExternal: (entry: Entry, media: LocalMedia?, chooser: Boolean) -> Unit,
) {
    Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        val pager = rememberPagerState(initialPage = set.start) { set.items.size }
        val loads = remember(set) { mutableStateMapOf<String, PageLoad>() }
        val retries = remember(set) { mutableStateMapOf<String, Int>() }
        var chrome by remember { mutableStateOf(true) }
        var zoomed by remember { mutableStateOf(false) }
        val focus = remember { FocusRequester() }
        val scope = rememberCoroutineScope()
        fun step(delta: Int) {
            val target = (pager.currentPage + delta).coerceIn(0, set.items.lastIndex)
            if (target != pager.currentPage) scope.launch { pager.animateScrollToPage(target) }
        }
        LaunchedEffect(Unit) { focus.requestFocus() }
        LaunchedEffect(pager.currentPage) { zoomed = false }

        Box(
            Modifier
                .fillMaxSize()
                .background(Color.Black)
                .focusRequester(focus)
                .focusable()
                .onPreviewKeyEvent { event ->
                    if (event.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                    when (event.key) {
                        Key.DirectionLeft, Key.PageUp -> {
                            step(-1)
                            true
                        }
                        Key.DirectionRight, Key.PageDown -> {
                            step(1)
                            true
                        }
                        else -> false
                    }
                },
        ) {
            HorizontalPager(
                state = pager,
                modifier = Modifier.fillMaxSize(),
                beyondViewportPageCount = 1,
                userScrollEnabled = !zoomed,
                key = { set.items[it].location },
            ) { page ->
                val entry = set.items[page]
                val active = page == pager.settledPage
                // Neighbor pictures load ahead; videos and audio only once they are shown.
                val wanted = set.local || entry.kind == "image" || active
                val retry = retries[entry.location] ?: 0
                LaunchedEffect(entry.location, wanted, retry) {
                    if (!wanted || loads[entry.location].isUsable()) return@LaunchedEffect
                    loads[entry.location] = PageLoad.Loading()
                    try {
                        val media = loadMedia(entry, set.local) { taskId ->
                            loads[entry.location] = PageLoad.Loading(taskId)
                        }
                        loads[entry.location] = PageLoad.Ready(media)
                    } catch (e: CoreException) {
                        loads[entry.location] = PageLoad.Failed(e.message ?: e.kind)
                    } catch (e: CancellationException) {
                        if (loads[entry.location] is PageLoad.Loading) loads.remove(entry.location)
                        throw e
                    }
                }
                val openHere = { media: LocalMedia? -> onOpenExternal(entry, media, false) }
                when (val load = loads[entry.location]) {
                    is PageLoad.Ready -> when (entry.kind) {
                        "image" -> ImagePage(
                            media = load.media,
                            onTap = { chrome = !chrome },
                            onZoomed = { if (page == pager.currentPage) zoomed = it },
                            unsupported = {
                                Message("Keine Vorschau für dieses Format.", "Mit App öffnen") { openHere(load.media) }
                            },
                        )
                        else -> PlayerPage(
                            media = load.media,
                            name = entry.name,
                            audio = entry.kind == "audio",
                            active = active,
                            controlsVisible = chrome,
                            onTap = { chrome = !chrome },
                            failed = { message -> Message(message, "Mit App öffnen") { openHere(load.media) } },
                        )
                    }
                    is PageLoad.Failed -> Message(load.message, "Erneut versuchen") {
                        retries[entry.location] = retry + 1
                    }
                    is PageLoad.Loading -> Loading(load.taskId)
                    null -> Loading(null)
                }
            }
            if (chrome) {
                val entry = set.items[pager.currentPage]
                TopBar(
                    title = entry.name,
                    position = "${pager.currentPage + 1} / ${set.items.size}",
                    onBack = onClose,
                    onOpen = { chooser ->
                        val media = (loads[entry.location] as? PageLoad.Ready)?.media
                        onOpenExternal(entry, media, chooser)
                    },
                    modifier = Modifier.align(Alignment.TopCenter),
                )
            }
        }
    }
}

@Composable
private fun TopBar(
    title: String,
    position: String,
    onBack: () -> Unit,
    onOpen: (chooser: Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    var menu by remember { mutableStateOf(false) }
    Row(
        modifier
            .fillMaxWidth()
            .background(Color.Black.copy(alpha = 0.55f))
            .padding(horizontal = 4.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onClick = onBack) { SeIcon(R.drawable.ic_arrow_back, "Zurück", tint = Color.White) }
        Column(Modifier.weight(1f)) {
            Text(
                title,
                color = Color.White,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(position, color = Color.White.copy(alpha = 0.8f), style = MaterialTheme.typography.labelMedium)
        }
        Box {
            IconButton(onClick = { menu = true }) { SeIcon(R.drawable.ic_more_vert, "Weitere Aktionen", tint = Color.White) }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                DropdownMenuItem(text = { Text("In App öffnen") }, onClick = {
                    menu = false
                    onOpen(false)
                })
                DropdownMenuItem(text = { Text("Öffnen mit…") }, onClick = {
                    menu = false
                    onOpen(true)
                })
            }
        }
    }
}

/** Download of a remote medium: honest progress when the task reports sizes. */
@Composable
private fun Loading(taskId: String?) {
    val tasks by Core.tasks.collectAsStateWithLifecycle()
    val task = taskId?.let { id -> tasks.firstOrNull { it.id == id } }
    Column(
        Modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        CircularProgressIndicator()
        val detail = if (task != null && task.totalBytes > 0) {
            " ${Format.size(task.doneBytes)} von ${Format.size(task.totalBytes)}"
        } else {
            ""
        }
        Text("Wird geladen…$detail", color = Color.White, modifier = Modifier.padding(16.dp))
    }
}

@Composable
private fun Message(text: String, action: String, onAction: () -> Unit) {
    Column(
        Modifier.fillMaxSize().padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        SeIcon(R.drawable.ic_error, null, tint = Color.White)
        Text(text, color = Color.White, textAlign = TextAlign.Center, modifier = Modifier.padding(16.dp))
        Button(onClick = onAction) { Text(action) }
    }
}
