package app.smartexplorer.android.ui.screen

import android.graphics.Bitmap
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
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
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.gestures.detectTransformGestures
import kotlin.math.roundToInt
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
    var bitmapFrame by remember { mutableStateOf<ScreenPresentedFrame?>(null) }
    var pictures by remember { mutableIntStateOf(0) }
    var failure by remember { mutableStateOf<String?>(null) }
    var silent by remember { mutableStateOf(false) }
    var nativePixels by remember { mutableStateOf(false) }
    var hiddenSinceFrame by remember { mutableStateOf(false) }
    var transformTick by remember { mutableIntStateOf(0) }
    val transform = remember { ViewTransform { transformTick++ } }
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
        repeat(2) { withFrameNanos { } }
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
    LaunchedEffect(session) { bitmap = null; bitmapFrame = null; pictures = 0 }
    LaunchedEffect(nativePixels, bitmap?.width, bitmap?.height) { transform.nativePixels(if (nativePixels) bitmap?.width else null, if (nativePixels) bitmap?.height else null) }
    LaunchedEffect(intervalMs, session) {
        session?.setMode("preview",fps=if(intervalMs<=500) 2 else 1)
    }
    val ui by remember(session) { session?.state ?: MutableStateFlow(ScreenSessionState()) }.collectAsState()
    LaunchedEffect(ui.inputReady) { if (ui.inputReady) hiddenSinceFrame = false }
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
        val activeDecoder = java.util.concurrent.atomic.AtomicBoolean(true)
        val main = android.os.Handler(android.os.Looper.getMainLooper())
        val decoder = ScreenDecoder(
            sessionId = active.id,
            source = backend.pictures(active.id),
            streamOf = { gop -> active.state.value.streams[gop] },
            surface = { null },
            presentationEpoch = active::presentationEpoch,
            bufferBudget = {active.state.value.bufferBudget},
            pullBuffers = active.decoderBuffers,
            listener = object : ScreenDecoder.Listener {
                override fun onSize(width: Int, height: Int) = Unit
                override fun onBitmap(picture: Bitmap) = Unit
                override fun onBitmapFrame(picture: Bitmap, frame: ScreenPresentedFrame) {
                    // Every picture is decoded; only selected immutable snapshots can be acknowledged.
                    if (!activeDecoder.get() || !active.isDisplayable(frame) || received++ % skip != 0) return
                    if(!active.beginBitmapSnapshot()) return
                    val snapshot = try {ScreenScaling.snapshot(picture,active.state.value.bufferBudget)} catch(error:Throwable) {
                        active.endBitmapSnapshot();throw error
                    }
                    main.post {
                        try {
                        if (!activeDecoder.get() || !active.isDisplayable(frame)) return@post
                        if (snapshot == null) {
                            failure = ScreenScaling.COPY_ERROR; bitmap = null; bitmapFrame = null
                            active.invalidateDisplay(false)
                        } else {
                            if (failure == ScreenScaling.COPY_ERROR) failure = null
                            bitmapFrame = frame
                            bitmap = snapshot
                            hiddenSinceFrame = false
                            pictures++
                        }
                        } finally {active.endBitmapSnapshot()}
                    }
                }
                override fun onPicture() = Unit
                override fun onHybridUnavailable(gopId: Long, reason: String) {
                    if (activeDecoder.get()) active.hybridUnavailable(gopId, reason, "preview")
                }
                override fun onDecodeFailure(consecutive: Int) { active.invalidateDisplay(false); active.call { keyframe(it) } }
                override fun onEnded() {
                    main.post { if (activeDecoder.get()) active.invalidateDisplay(false) }
                }
            },
        )
        decoder.start()
        onDispose { activeDecoder.set(false); decoder.stop(); active.invalidateDisplay(false) }
    }

    val ended = ui.ended
    val text = when {
        gate == TileGate.Offline -> stringResource(R.string.screen_tile_offline)
        gate == TileGate.Unsupported -> stringResource(R.string.screen_tile_unsupported)
        gate == TileGate.Off -> stringResource(R.string.screen_tile_off)
        gate == TileGate.Failed -> failure.orEmpty()
        ended != null -> ScreenTexts.endText(context, ended.code, ended.detail)
        ui.errorCode != null -> ScreenTexts.errorText(context, ui.errorCode.orEmpty(), ui.errorDetail)
        failure == ScreenScaling.COPY_ERROR -> failure.orEmpty()
        ui.codecFallback != null -> ui.codecFallback.orEmpty()
        hiddenSinceFrame && bitmapFrame?.let { session?.isDisplayable(it) } != true -> "Warte auf ein vollständiges aktuelles Bild …"
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
            BoxWithConstraints(Modifier.fillMaxWidth().aspectRatio(16f / 9f).background(Color.Black).clipToBounds().drawWithContent { ScreenScaling.observeRenderer(drawContext.canvas.nativeCanvas); drawContent() }, contentAlignment = Alignment.Center) {
                @Suppress("UNUSED_VARIABLE") val drawnFor = transformTick
                val vw = constraints.maxWidth.toFloat()
                val vh = constraints.maxHeight.toFloat()
                val shownSize = bitmap
                val sw = shownSize?.width ?: 0
                val sh = shownSize?.height ?: 0
                val aspect = if (sw > 0 && sh > 0) sw.toFloat()/sh else 16f/9f
                val (w,h) = transform.contentSize(vw,vh,aspect)
                val left = ((vw-w)/2+transform.panX).roundToInt().toFloat()
                val top = ((vh-h)/2+transform.panY).roundToInt().toFloat()
                Box(Modifier.matchParentSize().screenVisibility(Rect(left,top,left+w,top+h),sw,sh) { facts ->
                    if (adoptedBy !== session) {
                        if (!facts.visible) hiddenSinceFrame = true
                        session?.publishVisibility(facts)
                    }
                })
                val shown = bitmap
                val renderingSession=session
                if (shown != null && renderingSession != null && text.isEmpty() && (!hiddenSinceFrame || bitmapFrame?.let { session?.isDisplayable(it) } == true)) {
                    val placed = Modifier.align(Alignment.TopStart)
                        .pointerInput(nativePixels) { if (nativePixels) detectTransformGestures { _,pan,_,_ -> transform.panBy(pan.x,pan.y,vw,vh,aspect) } }
                    ScreenBitmap(shown,pictures,w.roundToInt(),h.roundToInt(),placed,vw.roundToInt(),vh.roundToInt(),left.roundToInt(),top.roundToInt(),
                        identity = bitmapFrame, scaler = renderingSession.scaleWorker,
                        onPresented = { frame -> if (adoptedBy !== session) session?.confirmPresented(frame) })
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
                TextButton(onClick = { nativePixels = !nativePixels }) { Text(if (nativePixels) "Einpassen" else "1:1") }
                Text(device.name, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                Box(Modifier.size(10.dp).clip(CircleShape).background(dot))
            }
        }
    }
}
