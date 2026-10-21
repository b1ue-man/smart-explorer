package app.smartexplorer.android.ui.screen

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.DecoderOffer
import app.smartexplorer.android.api.ScreenDevice
import app.smartexplorer.android.core.CoreException
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow

/** The long edge of a preview picture in pixels (C6). */
private const val PREVIEW_EDGE = 320

/** A tile without a picture this long after the device accepted shows „Keine Antwort“ (twice the longest interval of 5 s). */
private const val NO_ANSWER_MS = 10_000L

/** What a tile shows before and instead of a picture, from the probe. */
private enum class TileGate { Probing, Offline, Unsupported, Off, Ask, Allow, Failed }

/**
 * One tile of the group view (SCR-M9, U28-9): the probe of the device decides what it shows – a
 * preview session (`allow`), the button „Ansehen anfragen“ (`ask`) or a text. The preview is a
 * `tiles` session of the long edge [PREVIEW_EDGE]; [intervalMs] 2 s and 5 s ask the host for one
 * picture a second and show every second or fifth. Tapping calls [onOpen] with the preview session
 * (only with `allow`), which the full view raises to the full stream. A tile whose session [adoptedBy]
 * a full view holds stops decoding until it is given back.
 */
@Composable
fun ScreenPreviewTile(
    device: ScreenDevice,
    intervalMs: Int,
    adoptedBy: ScreenSession?,
    onOpen: (ScreenDevice, ScreenSession?) -> Unit,
) {
    val context = LocalContext.current
    val backend = LocalScreenBackend.current
    var gate by remember { mutableStateOf(TileGate.Probing) }
    var session by remember { mutableStateOf<ScreenSession?>(null) }
    var bitmap by remember { mutableStateOf<Bitmap?>(null) }
    var pictures by remember { mutableIntStateOf(0) }
    var failure by remember { mutableStateOf<String?>(null) }
    var silent by remember { mutableStateOf(false) }
    val target = remember(device) { device.toTarget() }
    val skip = (intervalMs / 1000).coerceAtLeast(1)

    LaunchedEffect(device.online) {
        gate = TileGate.Probing
        if (!device.online) {
            gate = TileGate.Offline
            return@LaunchedEffect
        }
        gate = try {
            val probe = backend.probe(target)
            when {
                !probe.supported -> TileGate.Unsupported
                probe.view == "allow" -> TileGate.Allow
                probe.view == "ask" -> TileGate.Ask
                else -> TileGate.Off
            }
        } catch (e: CoreException) {
            failure = e.message
            TileGate.Failed
        }
    }
    LaunchedEffect(gate) {
        if (gate != TileGate.Allow) return@LaunchedEffect
        // One connection per tile: a probe that answers `allow` again (the device list follows the
        // Share status) keeps the running session; only an ended one is closed and replaced.
        val running = session
        if (running != null && running.state.value.ended == null) return@LaunchedEffect
        running?.close()
        session = null
        val opened = ScreenSession(target, backend)
        try {
            opened.open("preview", null, PREVIEW_EDGE, PREVIEW_EDGE, if (intervalMs <= 500) 2 else 1, listOf(DecoderOffer(codec = "tiles")))
            session = opened
        } catch (e: CoreException) {
            failure = e.message
            gate = TileGate.Failed
        }
    }
    DisposableEffect(Unit) { onDispose { session?.close() } }
    LaunchedEffect(intervalMs, session) {
        session?.call { setMode(it, "preview", fps = if (intervalMs <= 500) 2 else 1) }
    }
    val ui by remember(session) { session?.state ?: MutableStateFlow(ScreenSessionState()) }.collectAsState()
    LaunchedEffect(ui.accepted, pictures) {
        silent = false
        if (ui.accepted && pictures == 0) {
            delay(NO_ANSWER_MS)
            silent = true
        }
    }
    DisposableEffect(session, adoptedBy === session) {
        val active = session
        if (active == null || adoptedBy === active) return@DisposableEffect onDispose { }
        var received = 0
        val decoder = ScreenDecoder(
            sessionId = active.id,
            source = backend.pictures(active.id),
            streamOf = { gop -> active.state.value.streams[gop] },
            surface = { null },
            listener = object : ScreenDecoder.Listener {
                override fun onSize(width: Int, height: Int) = Unit
                override fun onBitmap(picture: Bitmap) {
                    // Every picture is decoded (later ones build on earlier ones); only some are shown.
                    if (received++ % skip == 0) {
                        bitmap = picture
                        pictures++
                    }
                }
                override fun onPicture() = Unit
                override fun onDecodeFailure(consecutive: Int) = Unit
                override fun onEnded() = Unit
            },
        )
        decoder.start()
        onDispose { decoder.stop() }
    }

    val ended = ui.ended
    val text = when {
        gate == TileGate.Offline -> stringResource(R.string.screen_tile_offline)
        gate == TileGate.Unsupported -> stringResource(R.string.screen_tile_unsupported)
        gate == TileGate.Off -> stringResource(R.string.screen_tile_off)
        gate == TileGate.Failed -> failure.orEmpty()
        ended != null -> ScreenTexts.endText(context, ended.code, ended.detail)
        ui.errorCode != null -> ScreenTexts.errorText(context, ui.errorCode.orEmpty(), ui.errorDetail)
        silent && bitmap == null -> stringResource(R.string.screen_tile_no_answer)
        bitmap == null && gate == TileGate.Allow -> ScreenTexts.stateText(context, ui.state, ui.reason).ifEmpty { stringResource(R.string.screen_tile_waiting) }
        else -> ""
    }
    val dot = when {
        bitmap != null && ended == null -> Color(0xFF2E7D32)
        gate == TileGate.Probing || gate == TileGate.Allow && ended == null && ui.errorCode == null -> Color(0xFFF9A825)
        else -> Color(0xFFC62828)
    }
    Card(Modifier.fillMaxWidth().clickable(enabled = gate == TileGate.Allow || gate == TileGate.Ask) {
        onOpen(device, session.takeIf { gate == TileGate.Allow && ui.accepted })
    }) {
        Column {
            Box(Modifier.fillMaxWidth().aspectRatio(16f / 9f).background(Color.Black), contentAlignment = Alignment.Center) {
                val shown = bitmap
                if (shown != null && text.isEmpty()) {
                    key(pictures) { Image(shown.asImageBitmap(), contentDescription = null, contentScale = ContentScale.Fit) }
                } else {
                    Text(
                        if (gate == TileGate.Ask) stringResource(R.string.screen_tile_ask) else text,
                        color = Color.White,
                        style = MaterialTheme.typography.bodySmall,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.padding(8.dp),
                    )
                }
            }
            Row(Modifier.padding(8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(device.name, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                Box(Modifier.size(10.dp).clip(CircleShape).background(dot))
            }
        }
    }
}
