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
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
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
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.currentStateAsState
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

/** The highest preview rate of the tiles' intervals (500 ms): the hello names it, `SET_MODE` sets the chosen one. */
private const val PREVIEW_MAX_FPS = 2

/**
 * Why a preview cannot run under [stored]; `null` when it can. A preview draws bitmaps without a Surface for
 * MediaCodec: only the tiles, decoded in software, run there (Codex 11/9).
 */
internal fun previewRefusal(stored: ViewerCodecChoice): String? = when {
    stored.codec in ViewerCodecChoice.VIDEO_CODECS ->
        "Keine Vorschau: Codec „${ScreenCodecNotices.label(stored.codec)}“ gewählt; die Vorschau zeigt nur verlustfreie Kacheln. Der Codec gilt in der Vollansicht."
    stored.decoder == ViewerCodecChoice.DECODER_HARDWARE ->
        "Keine Vorschau: „Dekodierer: Hardware“ gewählt; die Kacheln der Vorschau werden in Software dekodiert. Die Wahl gilt in der Vollansicht."
    else -> null
}

/** What a tile shows before and instead of a picture, from the probe. */
private enum class TileGate { Probing, Offline, Unsupported, Off, Ask, Allow, Failed }

/**
 * One tile of the group view (SCR-M9, U28-9): the probe of the device decides what it shows – a
 * preview session (`allow`), the button „Ansehen anfragen“ (`ask`) or a text. The preview is a
 * `tiles` session of the long edge [PREVIEW_EDGE]; [intervalMs] 2 s and 5 s ask the host for one
 * picture a second and show every second or fifth. Tapping calls [onOpen] with the preview session
 * (only with `allow`), which the full view raises to the full stream. A tile whose session [adoptedBy]
 * a full view holds stops decoding until it is given back. Below STARTED (app in the background) a
 * tile closes its own session – one peer connection and one decoder thread each – and opens it
 * again on the return, as on the first opening; a session a full view adopted stays with it.
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
    // The stored „Codec“, „Dekodierer“ and „Höchste Datenrate“ (Codex 2/15): the rate binds the preview too. Observed,
    // so a choice stored in the full view reaches the tiles that keep running (round 12, B2 / Codex 12/9).
    val stored by remember { ScreenViewPrefs.observe(context) }.collectAsState()
    // A preview draws bitmaps only (no Surface for MediaCodec): it can run the lossless tiles, decoded in software.
    // A named video codec or „Dekodierer: Hardware“ cannot hold there; the tile says so and opens no session instead
    // of showing the tiles against the choice (Codex 11/9). The full view keeps the choice.
    val previewRefused = previewRefusal(stored)
    // The interval in force, not the one of the decoder's start (Codex 11/13).
    val interval by rememberUpdatedState(intervalMs.toLong())
    // A dead decoding thread is started again a few times, its cause shown (Codex 11/11, as the full view). Counted per
    // session: a new session starts its decoder once, not a second time when a reset of the count followed (round 12, B4).
    var decoderRestarts by remember(session) { mutableIntStateOf(0) }
    // A picture that came before the interval's end waits for it (round 12, Codex 12/13): the newest one is shown then,
    // so the last change of a screen that stands still afterwards is not lost. `nextShowAt` is the end (uptime ms).
    var waiting by remember(session) { mutableStateOf<Pair<Bitmap, ScreenPresentedFrame>?>(null) }
    var nextShowAt by remember(session) { mutableLongStateOf(0L) }

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
    val lifecycleState by LocalLifecycleOwner.current.lifecycle.currentStateAsState()
    val started = lifecycleState.isAtLeast(Lifecycle.State.STARTED)
    // Keyed by the adoption of this tile's own session too (round 13, V1): a choice the preview cannot hold, stored in
    // the full view that adopted the session, leaves it running there; given back when the full view closes, it ends
    // here then. A full view over another tile changes nothing here (no session of this tile: no new key).
    LaunchedEffect(gate, started, previewRefused, adoptedBy != null && adoptedBy === session) {
        // In the background, and under a choice the preview cannot hold (stored later in the full view, round 12, B2), a
        // tile's own session ends; the tile names the reason (`previewRefused`).
        if (!started || previewRefused != null) {
            val running = session
            if (running != null && adoptedBy !== running) {
                running.close()
                session = null
            }
            return@LaunchedEffect
        }
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
            // A preview offers the tiles only, under the stored choice („Automatisch“ or „Verlustfrei“, decoder
            // „Automatisch“ or „Software“; `previewRefusal`) and „Höchste Datenrate“. The hello names the highest
            // preview rate (two pictures a second, 500 ms): the host keeps every later `SET_MODE` below it, so the
            // interval chosen below acts whichever one the tile opened with (Codex 11/13).
            opened.open("preview", null, PREVIEW_EDGE, PREVIEW_EDGE, PREVIEW_MAX_FPS, listOf(DecoderOffer(codec = "tiles")), stored)
            session = opened
        } catch (e: CoreException) {
            failure = e.message
            gate = TileGate.Failed
        }
    }
    DisposableEffect(Unit) { onDispose { session?.close() } }
    LaunchedEffect(session) { bitmap = null; bitmapFrame = null; pictures = 0 }
    // „Datenrate“, „Verlustfrei“ and „Übertragung“ stored while the session runs go live (round 12, B2; user
    // 2026-11-25): the rate as `RATE_CEILING`, the lossless tiles as `SET_MODE`, „Übertragung“ as `SET_DELIVERY` to a host
    // that names it in its statistics. A session a full view holds gets them from there.
    val applied = remember(session) { arrayOf<Any?>(session?.openedMaxRateBps, session?.choice?.lossless, session?.choice?.delivery) }
    LaunchedEffect(session, adoptedBy === session, stored.maxRateBps, stored.lossless, stored.delivery) {
        val running = session ?: return@LaunchedEffect
        if (adoptedBy === running || previewRefused != null) return@LaunchedEffect
        if (applied[0] != stored.maxRateBps) { applied[0] = stored.maxRateBps; running.setMaxRate(stored.maxRateBps) }
        if (applied[2] != stored.delivery) { applied[2] = stored.delivery; running.setDelivery(stored.delivery) }
        if (applied[1] != stored.lossless) {
            applied[1] = stored.lossless
            running.chooseCodec(stored.codec)
            running.setMode("preview", lossless = stored.lossless)
        }
    }
    /** Shows [picture] of [frame] now; the next one waits for the interval. */
    fun show(picture: Bitmap, frame: ScreenPresentedFrame) {
        if (failure == ScreenScaling.COPY_ERROR || failure?.startsWith(DECODER_ENDED) == true) failure = null
        bitmapFrame = frame
        bitmap = picture
        hiddenSinceFrame = false
        pictures++
        nextShowAt = android.os.SystemClock.uptimeMillis() + interval
    }
    LaunchedEffect(waiting) {
        val (picture, frame) = waiting ?: return@LaunchedEffect
        delay((nextShowAt - android.os.SystemClock.uptimeMillis()).coerceAtLeast(0L))
        waiting = null
        val active = session ?: return@LaunchedEffect
        if (active.isDisplayable(frame) || active.awaitsAck(frame)) show(picture, frame)
    }
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
    DisposableEffect(session, adoptedBy === session, decoderRestarts) {
        val active = session
        if (active == null || adoptedBy === active) return@DisposableEffect onDispose { }
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
                    // Every picture is decoded; only selected immutable snapshots can be acknowledged. A picture that
                    // came before the host's visibility answer is kept (`awaitsAck`): of a static desktop it is the
                    // only one, and the answer decides by its first frame whether it is shown (Codex 11/10).
                    if (!activeDecoder.get() || !(active.isDisplayable(frame) || active.awaitsAck(frame))) return
                    if(!active.beginBitmapSnapshot()) return
                    val snapshot = try {ScreenScaling.snapshot(picture,active.state.value.bufferBudget)} catch(error:Throwable) {
                        active.endBitmapSnapshot();throw error
                    }
                    main.post {
                        try {
                        if (!activeDecoder.get() || !(active.isDisplayable(frame) || active.awaitsAck(frame))) return@post
                        if (snapshot == null) {
                            failure = ScreenScaling.COPY_ERROR; bitmap = null; bitmapFrame = null
                            active.invalidateDisplay(false)
                        } else if (android.os.SystemClock.uptimeMillis() >= nextShowAt) {
                            waiting = null
                            show(snapshot, frame)
                        } else {
                            // Within the interval: the newest picture waits for its end (Codex 12/13).
                            waiting = snapshot to frame
                        }
                        } finally {active.endBitmapSnapshot()}
                    }
                }
                override fun onPicture() = Unit
                override fun onHybridUnavailable(gopId: Long, reason: String) {
                    if (activeDecoder.get()) active.hybridUnavailable(gopId, reason, "preview")
                }
                override fun onDecodeFailure(consecutive: Int) { active.invalidateDisplay(false); active.call { keyframe(it) } }
                override fun onAborted(reason: String) {
                    main.post {
                        if (!activeDecoder.get()) return@post
                        failure = "$DECODER_ENDED: $reason"
                        if (decoderRestarts < MAX_DECODER_RESTARTS) decoderRestarts++
                    }
                }
                override fun onEnded() {
                    main.post { if (activeDecoder.get()) active.invalidateDisplay(false) }
                }
            },
        )
        decoder.start()
        // A decoder started in the middle of a picture group (a restart) needs a key frame.
        if (decoderRestarts > 0 && active.state.value.stream != null) active.call { keyframe(it) }
        onDispose { activeDecoder.set(false); waiting = null; decoder.stop(); active.invalidateDisplay(false) }
    }

    val ended = ui.ended
    val text = when {
        gate == TileGate.Offline -> stringResource(R.string.screen_tile_offline)
        gate == TileGate.Unsupported -> stringResource(R.string.screen_tile_unsupported)
        gate == TileGate.Off -> stringResource(R.string.screen_tile_off)
        gate == TileGate.Failed -> failure.orEmpty()
        gate == TileGate.Allow && previewRefused != null -> previewRefused
        ended != null -> ScreenTexts.endText(context, ended.code, ended.detail)
        ui.errorCode != null -> ScreenTexts.errorText(context, ui.errorCode.orEmpty(), ui.errorDetail)
        failure?.startsWith(DECODER_ENDED) == true -> failure.orEmpty()
        failure == ScreenScaling.COPY_ERROR -> failure.orEmpty()
        ui.codecFallback != null -> ui.codecFallback.orEmpty()
        hiddenSinceFrame && bitmapFrame?.let { session?.isDisplayable(it) } != true -> "Warte auf ein vollständiges aktuelles Bild …"
        silent && bitmap == null -> stringResource(R.string.screen_tile_no_answer)
        bitmap == null && gate == TileGate.Allow -> ScreenTexts.stateText(context, ui.state, ui.reason).ifEmpty { stringResource(R.string.screen_tile_waiting) }
        else -> ""
    }
    val dot = when {
        failure?.startsWith(DECODER_ENDED) == true || gate == TileGate.Allow && previewRefused != null -> Color(0xFFC62828)
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
                // The host's visibility answer decides about a picture held before it (Codex 11/10): read here so
                // the box draws again when it comes, also when it names the same first frame again (round 12, B5).
                @Suppress("UNUSED_VARIABLE") val ackedFor = ui.visibilityAcks
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
                // A picture held before the host's visibility answer shows once the answer names it (`isDisplayable`).
                val frameNow = bitmapFrame
                val displayable = frameNow != null && session?.isDisplayable(frameNow) == true
                val waitsForAck = frameNow != null && !displayable && session?.awaitsAck(frameNow) == true
                if (shown != null && renderingSession != null && text.isEmpty() && !waitsForAck && (!hiddenSinceFrame || displayable)) {
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
