package app.smartexplorer.android.ui.screen

import android.graphics.Bitmap
import android.view.KeyEvent
import android.view.PointerIcon
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.ViewConfiguration
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
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
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.currentStateAsState
import app.smartexplorer.android.R
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.ScreenCursorEvent
import app.smartexplorer.android.ui.login.LoginViewScreen
import app.smartexplorer.android.ui.login.PairedLogin
import java.util.concurrent.atomic.AtomicLong
import kotlin.math.ceil
import kotlin.math.roundToInt
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.launch
import kotlinx.coroutines.delay

/** Zero-width characters in front of the keyboard field, so deleting works on an empty field. */
private const val FIELD_PAD = 32

/** Keys of the automatic login switch (`login.autoSwitch`), one per open full view. */
private val autoLoginKeys = AtomicLong(0)

/**
 * The full view of one device (SCR-M9, U28-7): the picture in a full-screen dialog with zoom 1× to
 * 8× and panning, the bar of [ScreenTopBar], touch control while „Steuern“ is on and the screen
 * keyboard. [adopted] keeps the tile's small preview contract while an independently admitted
 * full session runs; closing this view restores the tile and its selected source.
 */
@Composable
fun ScreenViewScreen(target: ScreenTarget, title: String, onClose: () -> Unit, adopted: ScreenSession? = null, family: FamilySession? = null) {
    val context = LocalContext.current
    // The pointer that hides the phone's own pointer over the picture (API 24, TYPE_NULL: no bitmap).
    val hiddenPointer = remember(context) { PointerIcon.getSystemIcon(context, PointerIcon.TYPE_NULL) }
    val backend = LocalScreenBackend.current
    val scope = rememberCoroutineScope()
    val owned = remember { mutableListOf<ScreenSession>() }
    val switchJobs = remember { arrayOfNulls<Job>(1) }
    val viewerExtent = remember { intArrayOf(0,0) }
    var current by remember(family, adopted) { mutableStateOf(adopted) }
    var familyMenuOpen by remember { mutableStateOf(false) }
    var mode by remember { mutableStateOf(if (adopted != null) "preview" else if (target.control) "control" else "view") }
    var controlOn by remember { mutableStateOf(target.control) }
    var failure by remember { mutableStateOf<String?>(null) }
    // „Codec“, „Dekodierer“ and „Höchste Datenrate“ are kept between sessions (BK7b); the old switch „Verlustfrei“ is the codec `lossless`.
    var settings by remember { mutableStateOf(ViewSettings(source = target.source, choice = ScreenViewPrefs.load(context))) }
    // The MediaCodec decoder that runs the picture group (hardware or software), reported by the decoder thread.
    var decoderStage by remember { mutableStateOf<DecoderStage?>(null) }
    var zoomMode by remember { mutableStateOf(false) }
    var keyboardOn by remember { mutableStateOf(false) }
    var barVisible by remember { mutableStateOf(true) }
    var immersive by remember { mutableStateOf(true) }
    var nativePixels by remember { mutableStateOf(false) }
    var localChordHeld by remember { mutableStateOf(false) }
    val swallowedLocalKeys = remember { mutableSetOf<Int>() }
    var surface by remember { mutableStateOf<Surface?>(null) }
    var bitmap by remember { mutableStateOf<Bitmap?>(null) }
    var bitmapGop by remember { mutableStateOf<Long?>(null) }
    var bitmapFrame by remember { mutableStateOf<ScreenPresentedFrame?>(null) }
    var surfaceFrame by remember { mutableStateOf<ScreenPresentedFrame?>(null) }
    var sourceAfterGop by remember { mutableStateOf<Long?>(null) }
    var hiddenSinceFrame by remember { mutableStateOf(false) }
    // True while the host's pointer overlay is drawn over the picture (see the pointer effect below).
    var hostPointerShown by remember { mutableStateOf(false) }
    var renderReady by remember { mutableStateOf(false) }
    var pictures by remember { mutableIntStateOf(0) }
    var failures by remember { mutableIntStateOf(0) }
    var transformTick by remember { mutableIntStateOf(0) }
    val transform = remember { ViewTransform { transformTick++ } }
    val flush = remember { Channel<Unit>(Channel.CONFLATED) }
    val session = current
    val ui by remember(session) { session?.state ?: MutableStateFlow(ScreenSessionState()) }.collectAsState()
    val metrics = context.resources.displayMetrics
    val maxFps = ceil(runCatching { context.display?.refreshRate }.getOrNull() ?: 60f).toInt().coerceAtLeast(1)

    suspend fun openOwn(newMode: String) {
        val opened = ScreenSession(target, backend)
        try {
            opened.open(newMode, settings.source, viewerExtent[0].takeIf { it > 0 } ?: metrics.widthPixels, viewerExtent[1].takeIf { it > 0 } ?: metrics.heightPixels, maxFps, backend.offers(settings.choice), settings.choice)
        } catch (e: CoreException) {
            failure = e.message
            return
        }
        val previous = current
        owned.add(opened)
        current = opened
        if (previous === adopted && previous != null) {
            previous.publishVisibility(ScreenVisibilityFacts(false,Rect.Zero,Rect.Zero,Rect.Zero,0,0))
        }
        mode = newMode
        failure = null
        if (previous != null && previous !== adopted) {
            owned.remove(previous)
            previous.close()
        }
    }
    LaunchedEffect(Unit) {
        // The first black dialog draw proves the renderer limit without a remote image.
        repeat(2) { withFrameNanos { } }
        if (family != null) {
            try {
                family.open(viewerExtent[0].takeIf { it > 0 } ?: metrics.widthPixels, viewerExtent[1].takeIf { it > 0 } ?: metrics.heightPixels, maxFps, backend.offers())
                current = family.screen
            } catch (cancel: kotlinx.coroutines.CancellationException) { throw cancel
            } catch (error: Exception) { failure = error.message ?: "Fensterverbindung konnte nicht geöffnet werden." }
        } else if (adopted == null) {
            openOwn(mode)
        } else {
            // A preview owns its small immutable frame contract. Full view gets
            // its own actual budget, as control already did, and leaves the tile
            // available for the return path without widening that contract.
            openOwn(if(target.control) "control" else "view")
        }
    }

    val host = if (ui.platform == "android") InputHost.ANDROID else InputHost.DESKTOP
    val geometry = remember { floatArrayOf(1f, 1f, 1f) }
    val aspect = ui.stream?.let { it.width.toFloat() / it.height.coerceAtLeast(1) } ?: (metrics.widthPixels.toFloat() / metrics.heightPixels)
    val viewConfig = remember { ViewConfiguration.get(context) }
    val longPressMs = ViewConfiguration.getLongPressTimeout().toLong()
    val mapper = remember(host, ui.input) {
        ScreenInputMapper(
            host = host,
            config = InputConfig(longPressMs, viewConfig.scaledTouchSlop.toFloat(), viewConfig.scaledVerticalScrollFactor),
            toSource = { x, y -> transform.toSource(x, y, geometry[0], geometry[1], geometry[2]) },
            caps = ui.input,
            onZoom = { scale, fx, fy -> transform.zoomBy(scale, fx, fy, geometry[0], geometry[1], geometry[2]) },
        )
    }
    val mapperNow by rememberUpdatedState(mapper)
    val sessionNow by rememberUpdatedState(session)
    // Input goes out only in a session of mode control, which the device lets control, while nobody else holds it.
    val sending = !familyMenuOpen && controlOn && ui.canControl && mode == "control" && ui.holder != "other" && ui.inputReady && renderReady
    val sendingNow by rememberUpdatedState(sending)
    fun releaseInputs() = sessionNow?.sendInput(mapperNow.releaseAll())
    LaunchedEffect(Unit) {
        for (signal in flush) {
            withFrameNanos { }
            if (sendingNow) sessionNow?.sendInput(mapperNow.drain()) else releaseInputs()
        }
    }
    LaunchedEffect(sending, session?.id) {
        if (sending) session?.call { control(it, "take") } else releaseInputs()
    }
    LaunchedEffect(ui.inputReady) { if (ui.inputReady) hiddenSinceFrame = false }
    LaunchedEffect(ui.ended) { if (ui.ended != null) releaseInputs() }
    // E22: while the stream pauses with the secure desktop, the lock screen or no user session, the
    // paired device's login screen opens without a tap; it closes when the user session shows again.
    val autoLoginKey = remember { autoLoginKeys.incrementAndGet() }
    var autoLogin by remember { mutableStateOf<PairedLogin?>(null) }
    LaunchedEffect(ui.state, target.deviceId, target.relationId) {
        val (action, paired) = try { PairedLogin.observe(autoLoginKey, target.deviceId ?: target.relationId, ui.state) }
            catch (cancel: CancellationException) { throw cancel } catch (_: Exception) { "stay" to null } catch (_: LinkageError) { "stay" to null }
        when (action) {
            "enter_login" -> if (paired != null) { releaseInputs(); autoLogin = paired }
            "leave_login" -> autoLogin = null
        }
    }
    LifecycleEventEffect(Lifecycle.Event.ON_PAUSE) { releaseInputs() }
    DisposableEffect(Unit) {
        onDispose {
            // First what is pressed, then the sessions (the queued input goes out before a session ends).
            releaseInputs()
            switchJobs[0]?.cancel()
            owned.forEach { it.close() }
            if (family == null) adopted?.setMode("preview",settings.source)
            CoroutineScope(Dispatchers.IO).launch { try { PairedLogin.end(autoLoginKey) } catch (_: Throwable) { Unit } }
        }
    }
    val view = LocalView.current
    DisposableEffect(view) {
        view.keepScreenOn = true
        onDispose { view.keepScreenOn = false }
    }
    LaunchedEffect(session) { bitmap = null; bitmapGop = null; bitmapFrame = null; surfaceFrame = null; sourceAfterGop = null; renderReady = false; decoderStage = null }
    // The host's pointer overlay (ScreenCursorOverlay) is drawn only while its shape and position are visible,
    // and only in the own full view (family == null). Only then the phone's own pointer is hidden (plan 4.9).
    LaunchedEffect(session?.id, family) {
        val id = session?.id ?: 0L
        hostPointerShown = false
        if (id == 0L || family != null) return@LaunchedEffect
        val visibility = HostPointerVisibility()
        Core.events.filterIsInstance<ScreenCursorEvent>().filter { it.session == id }.collect { event ->
            hostPointerShown = visibility.update(event)
        }
    }
    LaunchedEffect(session, ui.accepted) {
        if(ui.accepted && session!=null && owned.any {it===session}) {
            session.setMode("full",settings.source,sharpness=settings.sharpness,lossless=settings.lossless)
        }
    }
    LaunchedEffect(immersive, barVisible) { if (immersive && barVisible) { delay(3000); barVisible = false } }
    LaunchedEffect(nativePixels, ui.stream?.width, ui.stream?.height) {
        transform.nativePixels(if (nativePixels) ui.stream?.width else null, if (nativePixels) ui.stream?.height else null)
    }
    // The decoder (a thread polling the session's pictures) runs only while the app is at least
    // STARTED; the session itself stays open, so the return needs no new connection and the new
    // decoder asks for a key frame below.
    val lifecycleState by LocalLifecycleOwner.current.lifecycle.currentStateAsState()
    val decoding = lifecycleState.isAtLeast(Lifecycle.State.STARTED)
    DisposableEffect(session, surface, decoding) {
        val active = session
        if (active == null || active.id == 0L || !decoding) return@DisposableEffect onDispose { }
        val activeDecoder = java.util.concurrent.atomic.AtomicBoolean(true)
        val main = android.os.Handler(android.os.Looper.getMainLooper())
        // The stream's own picture rate (decoded pictures per second) and the surface it is drawn on.
        val pictureRate = PictureRateMeter(System::nanoTime)
        val drawing = surface
        val decoder = ScreenDecoder(
            sessionId = active.id,
            source = backend.pictures(active.id),
            streamOf = { gop -> active.state.value.streams[gop] },
            surface = { surface },
            presentationEpoch = active::presentationEpoch,
            bufferBudget = {active.state.value.bufferBudget},
            pullBuffers = active.decoderBuffers,
            choice = { active.choice },
            listener = object : ScreenDecoder.Listener {
                override fun onSize(width: Int, height: Int) = Unit
                override fun onBitmap(picture: Bitmap) = Unit
                override fun onBitmapFrame(picture: Bitmap, frame: ScreenPresentedFrame) {
                    if (!activeDecoder.get() || !active.isDisplayable(frame)) return
                    if(!active.beginBitmapSnapshot()) return
                    val snapshot = try {ScreenScaling.snapshot(picture,active.state.value.bufferBudget)} catch(error:Throwable) {
                        active.endBitmapSnapshot();throw error
                    }
                    main.post {
                        try {
                        if (!activeDecoder.get() || !active.isDisplayable(frame)) return@post
                        if (snapshot == null) {
                            failure = ScreenScaling.COPY_ERROR; bitmap = null; bitmapFrame = null; renderReady = false
                            active.invalidateDisplay(false)
                        } else {
                            if (failure == ScreenScaling.COPY_ERROR) failure = null
                            sourceAfterGop = null
                            hiddenSinceFrame = false
                            bitmapGop = frame.gopId
                            bitmapFrame = frame
                            bitmap = snapshot
                            pictures++
                        }
                        } finally {active.endBitmapSnapshot()}
                    }
                }
                override fun onSurfaceFrame(frame: ScreenPresentedFrame) {
                    if (!activeDecoder.get() || !active.isDisplayable(frame)) return
                    val continues = surfaceFrameContinues(surfaceFrame, frame, bitmap != null)
                    // A Bitmap overlay belongs to the previous decoder family and must leave first.
                    bitmap = null; bitmapGop = null; bitmapFrame = null
                    sourceAfterGop = null; hiddenSinceFrame = false
                    surfaceFrame = frame
                    // Only a new picture group, display epoch or decoder family waits for its first
                    // drawn frame. Resetting on every frame turned `sending` off and on per frame:
                    // each time all held input was released, control re-taken and the touch
                    // handler swapped, which cut every drag and long press on the remote host.
                    if (!continues) renderReady = false
                    failures = 0
                }
                override fun onDecoderStage(codec: String, name: String, hardware: Boolean) {
                    main.post { if (activeDecoder.get()) decoderStage = DecoderStage(codec, name, hardware) }
                }
                override fun onPicture() {
                    // Called on the decoding thread for every picture; the rate goes to the surface on the main thread.
                    val hz = pictureRate.picture() ?: return
                    main.post { if (activeDecoder.get()) drawing?.setStreamFrameRate(hz) }
                }
                override fun onHybridUnavailable(gopId: Long, reason: String) {
                    if (activeDecoder.get()) active.hybridUnavailable(gopId, reason, "full")
                }
                override fun onDecodeFailure(consecutive: Int) {
                    active.invalidateDisplay(false)
                    main.post { if (activeDecoder.get()) { failures = consecutive; renderReady = false } }
                    active.call { keyframe(it) }
                }
                override fun onEnded() {
                    main.post { if (activeDecoder.get()) { active.invalidateDisplay(false); renderReady = false } }
                }
            },
        )
        decoder.start()
        // A decoder that starts in the middle of a picture group needs a key frame.
        active.call { keyframe(it) }
        onDispose { activeDecoder.set(false); decoder.stop(); active.invalidateDisplay(false); drawing?.setStreamFrameRate(0) }
    }

    fun applySettings(next: ViewSettings) {
        if (settings.source != next.source) { releaseInputs(); session?.invalidateDisplay(); sourceAfterGop = ui.stream?.gop; bitmap = null; bitmapFrame = null; surfaceFrame = null; renderReady = false }
        val previous = settings
        settings = next
        if (previous.choice != next.choice) ScreenViewPrefs.save(context, next.choice)
        // Another codec or decoder changes the negotiation: a new session (E6 of BK6). „Verlustfrei“ and the data rate go live.
        val opened = session?.choice
        if (opened != null && ViewerCodecChoice.needsReopen(opened, next.choice)) {
            releaseInputs()
            // P1: a host with `set_decoders_v1` changes codec and decoder within the session (`screen.viewSetDecoders`), so an
            // Android host does not ask for its screen again; an older host or a failed call opens a new session as before.
            val own = session
            switchJobs[0]?.cancel()
            switchJobs[0] = scope.launch {
                val prior = own?.prepareDecoderSwitch(next.choice)
                val switched = if (own != null && own.id != 0L) {
                    try {
                        val offers = backend.offers(next.choice).map {
                            Core.json.encodeToJsonElement(app.smartexplorer.android.api.DecoderOffer.serializer(), it)
                        }
                        val answer = Core.call("screen.viewSetDecoders", kotlinx.serialization.json.JsonObject(mapOf(
                            "session" to kotlinx.serialization.json.JsonPrimitive(own.id),
                            "codec" to kotlinx.serialization.json.JsonPrimitive(next.choice.codec),
                            "decoder" to kotlinx.serialization.json.JsonPrimitive(next.choice.decoder),
                            "decoders" to kotlinx.serialization.json.JsonArray(offers),
                        )))
                        ((answer as? kotlinx.serialization.json.JsonObject)?.get("switched") as? kotlinx.serialization.json.JsonPrimitive)?.content == "true"
                    } catch (cancel: CancellationException) {
                        if (prior != null && own.choice == next.choice) own.restoreChoice(prior)
                        throw cancel
                    } catch (_: Exception) {
                        false
                    } catch (_: LinkageError) {
                        false
                    }
                } else false
                if (settings.choice != next.choice) return@launch
                if (switched && own != null) {
                    // „Verlustfrei“ is a live switch beside the negotiation: leaving it for another codec turns it off here,
                    // because the host keeps a new pairing under „Verlustfrei“ until that switch goes off.
                    own.setMode("full", next.source, sharpness = next.sharpness, lossless = next.lossless)
                    if (previous.choice.maxRateBps != next.choice.maxRateBps) own.setMaxRate(next.choice.maxRateBps)
                } else {
                    if (own != null && prior != null && own.choice == next.choice) own.restoreChoice(prior)
                    openOwn(mode)
                }
            }
            return
        }
        session?.setMode("full",next.source,sharpness=next.sharpness,lossless=next.lossless)
        if (previous.choice.maxRateBps != next.choice.maxRateBps) session?.setMaxRate(next.choice.maxRateBps)
    }
    val ended = ui.ended
    val openedRate = session?.openedMaxRateBps?.takeIf { ui.rateSends == 0 }
    val info = chainInfoLine(ui.stream?.codec, decoderStage, ScreenChainInfo.rateNote(ui.rateCeiling, openedRate, settings.choice.maxRateBps, ui.rateAnswered))
    val status = when {
        ended != null -> ScreenTexts.endText(context, ended.code, ended.detail)
        failure != null -> failure.orEmpty()
        !ui.inputReady && controlOn -> "Warte auf vollständiges aktuelles Bild …"
        failures > 3 -> stringResource(R.string.screen_view_decode_failed)
        ui.blocked != null && sending -> ScreenTexts.blockedText(context, ui.blocked.orEmpty())
        ui.holder == "other" -> ScreenTexts.errorText(context, "control_busy")
        ui.errorCode != null -> ScreenTexts.errorText(context, ui.errorCode.orEmpty(), ui.errorDetail)
        else -> ScreenTexts.stateText(context, ui.state, ui.reason)
    }

    Dialog(
        onDismissRequest = { releaseInputs(); if (family != null) onClose() else if (immersive) { immersive = false; barVisible = true } else onClose() },
        properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false, dismissOnClickOutside = false),
    ) {
        ScreenImmersive(immersive)
        val rootFocus = remember { FocusRequester() }
        Box(
            Modifier
                .fillMaxSize()
                .background(Color.Black)
                .clipToBounds()
                .drawWithContent { ScreenScaling.observeRenderer(drawContext.canvas.nativeCanvas); drawContent() }
                .pointerInput(Unit) {
                    awaitPointerEventScope {
                        while (true) {
                            val event = awaitPointerEvent(PointerEventPass.Initial)
                            if (family == null && event.changes.any { !it.previousPressed && it.pressed && it.position.y < 32.dp.toPx() }) {
                                barVisible = true
                                event.changes.forEach { it.consume() }
                            }
                        }
                    }
                }
                .focusRequester(rootFocus)
                .focusable()
                .onPreviewKeyEvent { event ->
                    val native = event.nativeKeyEvent
                    val down = event.type == KeyEventType.KeyDown
                    if (localFullscreenKey(native)) {
                        if (!down) swallowedLocalKeys.remove(native.keyCode)
                        if (down && native.repeatCount == 0) {
                            releaseInputs()
                            localChordHeld = native.isCtrlPressed || native.isAltPressed || native.isShiftPressed
                            swallowedLocalKeys.add(native.keyCode)
                            immersive = if (native.keyCode == KeyEvent.KEYCODE_F11) !immersive else false
                            barVisible = true
                        }
                        return@onPreviewKeyEvent true
                    }
                    if (native.keyCode in swallowedLocalKeys) {
                        if (!down) swallowedLocalKeys.remove(native.keyCode)
                        return@onPreviewKeyEvent true
                    }
                    if (localChordHeld) {
                        localChordHeld = native.isCtrlPressed || native.isAltPressed || native.isShiftPressed
                        return@onPreviewKeyEvent true
                    }
                    val known = ScreenInputMapper.keyName(native.keyCode) != null
                    if (sending && known && native.repeatCount == 0) {
                        mapperNow.onKey(native.keyCode, event.type == KeyEventType.KeyDown, native.metaState)
                        flush.trySend(Unit)
                    }
                    sending && known
                },
        ) {
            // The open soft keyboard keeps the focus: taking it back for the picture
            // whenever input readiness returns (which the keyboard's own layout change
            // triggers) ended the text input session and closed the keyboard at once.
            LaunchedEffect(sending, keyboardOn) { if (sending && !keyboardOn) rootFocus.requestFocus() }
            BoxWithConstraints(Modifier.fillMaxSize()) {
                @Suppress("UNUSED_VARIABLE") val drawnFor = transformTick // zoom or pan changed: draw again
                val viewW = constraints.maxWidth.toFloat()
                val viewH = constraints.maxHeight.toFloat()
                viewerExtent[0] = constraints.maxWidth
                viewerExtent[1] = constraints.maxHeight
                geometry[0] = viewW
                geometry[1] = viewH
                geometry[2] = aspect
                val (w, h) = transform.contentSize(viewW, viewH, aspect)
                val left = ((viewW - w) / 2 + transform.panX).roundToInt().toFloat()
                val top = ((viewH - h) / 2 + transform.panY).roundToInt().toFloat()
                val density = LocalDensity.current
                val placed = Modifier
                    .offset { IntOffset(left.roundToInt(), top.roundToInt()) }
                    .wrapContentSize(Alignment.TopStart,unbounded = true)
                    .size(with(density) { w.toDp() }, with(density) { h.toDp() })
                val showFresh = !hiddenSinceFrame || bitmapFrame?.let { session?.isDisplayable(it) } == true
                val shown = bitmap.takeIf { bitmapGop == ui.stream?.gop && showFresh && bitmapFrame?.let { session?.isDisplayable(it) } == true }
                val familyImageMissing = family != null && shown == null && surfaceFrame?.let { session?.isDisplayable(it) } != true
                AndroidView(factory = { ctx -> surfaceView(ctx) { surface = it } },
                    update = { nativeView ->
                        ui.stream?.let { if (it.width > 0 && it.height > 0) nativeView.holder.setFixedSize(it.width,it.height) }
                        // While the host's pointer overlay is drawn, the phone's own pointer over the picture is hidden: one pointer.
                        nativeView.setPointerIcon(if (hostPointerShown) hiddenPointer else null)
                    },
                    modifier = placed.alpha(if (shown == null) 1f else 0f).drawWithContent {
                        drawContent()
                        if (shown == null && sourceAfterGop == null && showFresh) {
                            surfaceFrame?.let { frame ->
                                if (session?.isDisplayable(frame) == true) { session.confirmPresented(frame); renderReady = true }
                            }
                        }
                    })
                if (sourceAfterGop != null || !showFresh || familyImageMissing) Box(placed.background(Color.Black))
                if (shown != null && session != null) {
                    ScreenBitmap(shown,pictures,w.roundToInt(),h.roundToInt(),Modifier,viewW.roundToInt(),viewH.roundToInt(),left.roundToInt(),top.roundToInt(),
                        identity = bitmapFrame, scaler = session.scaleWorker, onPresented = { frame ->
                            if (session?.isDisplayable(frame) == true) { session.confirmPresented(frame); renderReady = true }
                        }) { ready -> if (!ready) renderReady = false }
                }

                // The host's pointer, when it travels beside the picture (plan 2026-11-14 P6).
                if (family == null) ScreenCursorOverlay(session?.id ?: 0L, left, top, w, h)

                val gestures = if (sending && !zoomMode) {
                    Modifier.mapperTouchInput(mapper, longPressMs) { flush.trySend(Unit) }
                } else {
                    Modifier
                        .pointerInput(Unit) { detectTapGestures(onTap = { barVisible = !barVisible }, onDoubleTap = { transform.reset() }) }
                        .pointerInput(Unit) {
                            detectTransformGestures { centroid, pan, zoom, _ ->
                                transform.zoomBy(zoom, centroid.x, centroid.y, viewW, viewH, aspect)
                                if (nativePixels || transform.zoom > 1f) transform.panBy(pan.x, pan.y, viewW, viewH, aspect)
                            }
                        }
                }
                Box(Modifier.fillMaxSize().screenVisibility(Rect(left,top,left+w,top+h),ui.stream?.width ?: 0,ui.stream?.height ?: 0) { facts ->
                    if (!facts.visible || facts.rects().isEmpty() && facts.sourceWidth > 0) {
                        hiddenSinceFrame = true
                        releaseInputs()
                    }
                    session?.publishVisibility(facts.copy(sourceKey = settings.source))
                }.then(gestures))
            }
            if (family == null && (barVisible || !immersive || keyboardOn)) {
                Box(Modifier.align(Alignment.TopCenter)) {
                    ScreenTopBar(
                        title = title,
                        status = status,
                        ui = ui,
                        controlOn = controlOn,
                        settings = settings,
                        zoomMode = zoomMode,
                        onControl = { on ->
                            controlOn = on
                            if (on && mode != "control") scope.launch { openOwn("control") }
                            if (!on) session?.call { control(it, "release") }
                        },
                        onSource = { applySettings(settings.copy(source = it)) },
                        onListWindows = { session?.call { listWindows(it) } },
                        onAction = { if (sending) mapper.action(it).also { flush.trySend(Unit) } },
                        onUsage = { if (sending) mapper.tapUsage(it).also { flush.trySend(Unit) } },
                        onKeyboard = { keyboardOn = !keyboardOn },
                        onZoomMode = { zoomMode = !zoomMode },
                        onSettings = { applySettings(it) },
                        onStatus = if (failures > 3 && !settings.lossless) ({ failures = 0; applySettings(settings.copy(choice = settings.choice.copy(codec = ViewerCodecChoice.CODEC_LOSSLESS))) }) else null,
                        onClose = { releaseInputs(); onClose() },
                        info = info,
                        nativePixels = nativePixels,
                        onNativePixels = { nativePixels = !nativePixels },
                        immersive = immersive,
                        onFullscreen = { releaseInputs(); immersive = !immersive; barVisible = true },
                    )
                }
            }
            if (family == null && immersive && barVisible) {
                TextButton(onClick = { releaseInputs(); immersive = false; barVisible = true },
                    modifier = Modifier.align(Alignment.BottomCenter).navigationBarsPadding().imePadding().padding(8.dp)) {
                    Text("Vollbild verlassen · Zurück / Randgeste",color = Color.White)
                }
            }
            if (ended != null) {
                Row(Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(16.dp)) {
                    if (family == null) TextButton(onClick = { scope.launch { openOwn(if (controlOn) "control" else "view") } }) {
                        Text(stringResource(R.string.screen_view_reconnect), color = Color.White)
                    }
                    TextButton(onClick = onClose) { Text(stringResource(R.string.screen_view_close), color = Color.White) }
                }
            }
            if (family != null) {
                failure?.let { Text(it, color = Color.White, modifier = Modifier.align(Alignment.Center).padding(24.dp)) }
                FamilyWindowMenu(family, onClose, onLocalUiOpen = { familyMenuOpen = it }, onKeyboard = { keyboardOn = !keyboardOn })
            }
            // The field stays while the keyboard is on; short input pauses
            // (new picture, visibility) must not close and reopen the IME.
            if (keyboardOn && controlOn && mode == "control") ScreenKeyboardField(mapper, flush)
        }
    }
    autoLogin?.let { paired ->
        LoginViewScreen(paired.target, paired.user, onClose = { autoLogin = null; scope.launch { runCatching { PairedLogin.failed(autoLoginKey) } } })
    }
}

/**
 * Whether surface frame [next] continues the frames already drawn: same session, picture group and
 * display epoch as [shown], and no bitmap of another decoder family on top. Only then does the
 * renderer's readiness stay; the frame is still confirmed when drawn.
 */
internal fun surfaceFrameContinues(shown: ScreenPresentedFrame?, next: ScreenPresentedFrame, bitmapShown: Boolean): Boolean =
    !bitmapShown && shown != null && shown.session == next.session && shown.gopId == next.gopId && shown.epoch == next.epoch

/** The surface the decoder renders on; [onSurface] hears when it exists and when it is gone (`null`). */
private fun surfaceView(context: android.content.Context, onSurface: (Surface?) -> Unit) = SurfaceView(context).apply {
    holder.addCallback(object : SurfaceHolder.Callback {
        override fun surfaceCreated(holder: SurfaceHolder) = onSurface(holder.surface)

        override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) = Unit

        override fun surfaceDestroyed(holder: SurfaceHolder) = onSurface(null)
    })
}

/**
 * The invisible text field that brings up the screen keyboard (U28-8): what is typed becomes `text`
 * events, deleting becomes backspace and the send key becomes enter. Padding in front of the text
 * lets deleting work on an empty field.
 */
@Composable
private fun ScreenKeyboardField(mapper: ScreenInputMapper, flush: Channel<Unit>) {
    val pad = "\u200B".repeat(FIELD_PAD)
    // The cursor starts and stays behind the padding: a field from a plain
    // String puts it at index 0, so every typed character landed in front of
    // the padding and was never sent (2026-10-27: phone typing never arrived).
    var field by remember { mutableStateOf(TextFieldValue(pad, TextRange(pad.length))) }
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(Unit) {
        focus.requestFocus()
        keyboard?.show()
    }
    DisposableEffect(Unit) { onDispose { keyboard?.hide() } }
    BasicTextField(
        value = field,
        onValueChange = { next ->
            val (removed, inserted) = keyboardEdit(field.text, next.text)
            repeat(removed) { mapper.tapKey("backspace") }
            if (inserted.isNotEmpty()) mapper.onText(inserted)
            if (removed > 0 || inserted.isNotEmpty()) flush.trySend(Unit)
            val length = next.text.length
            field = if (next.composition == null && (length < FIELD_PAD / 2 || length > FIELD_PAD * 8)) {
                TextFieldValue(pad, TextRange(pad.length))
            } else {
                next
            }
        },
        modifier = Modifier.size(1.dp).alpha(0f).focusRequester(focus),
        singleLine = true,
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
        keyboardActions = KeyboardActions(onSend = {
            mapper.tapKey("enter")
            flush.trySend(Unit)
        }),
    )
}

/**
 * Hands the picture rate of the stream to the surface: `Surface.setFrameRate` with
 * `FRAME_RATE_COMPATIBILITY_DEFAULT` (API 30, the app's minSdk), because a screen stream is no fixed-rate
 * video; `0` gives the choice back to the system.
 */
private fun Surface.setStreamFrameRate(hz: Int) {
    if (!isValid) return
    try {
        setFrameRate(hz.toFloat(), Surface.FRAME_RATE_COMPATIBILITY_DEFAULT)
    } catch (_: RuntimeException) {
        // The surface refuses the rate (IllegalArgumentException), was released meanwhile (IllegalStateException) or its
        // native call failed (RuntimeException): the system keeps its choice. The rate is a hint; it never ends the view.
    }
}

/**
 * Pictures per second of a decoded stream, counted in windows of one second. The first picture only
 * opens a window; a window that ends yields its rounded rate, so a stream that stops keeps its last rate.
 * Called from the decoding thread only.
 */
internal class PictureRateMeter(private val nowNs: () -> Long) {
    private var windowStart = -1L
    private var pictures = 0L

    /** Counts one picture; the rate of a window that ended with it, otherwise `null`. */
    fun picture(): Int? {
        val now = nowNs()
        if (windowStart < 0L) {
            windowStart = now
            return null
        }
        pictures++
        val elapsed = now - windowStart
        if (elapsed < WINDOW_NS) return null
        val rate = (pictures * NS_PER_S + elapsed / 2) / elapsed
        windowStart = now
        pictures = 0L
        return rate.toInt().takeIf { it > 0 }
    }

    private companion object {
        const val WINDOW_NS = 1_000_000_000L
        const val NS_PER_S = 1_000_000_000L
    }
}

/**
 * Whether [ScreenCursorOverlay] draws the host's pointer after the events seen so far: a shape and a
 * position are known, the position is visible and it names the current shape. Only then the phone's
 * own pointer over the picture is hidden, so exactly one pointer remains.
 */
internal class HostPointerVisibility {
    private var shape: ScreenCursorEvent.Shape? = null
    private var position: ScreenCursorEvent.Position? = null

    /** Records [event] and tells whether the overlay is drawn now. */
    fun update(event: ScreenCursorEvent): Boolean {
        when (event) {
            is ScreenCursorEvent.Shape -> shape = event
            is ScreenCursorEvent.Position -> position = event
        }
        val known = shape ?: return false
        val where = position ?: return false
        return where.visible && where.shapeId == known.id
    }
}
