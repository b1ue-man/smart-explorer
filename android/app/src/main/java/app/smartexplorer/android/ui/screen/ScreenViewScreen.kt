package app.smartexplorer.android.ui.screen

import android.graphics.Bitmap
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.ViewConfiguration
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
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
import androidx.compose.runtime.key
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import app.smartexplorer.android.R
import app.smartexplorer.android.core.CoreException
import kotlin.math.ceil
import kotlin.math.roundToInt
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch

/** Zero-width characters in front of the keyboard field, so deleting works on an empty field. */
private const val FIELD_PAD = 32

/**
 * The full view of one device (SCR-M9, U28-7): the picture in a full-screen dialog with zoom 1× to
 * 8× and panning, the bar of [ScreenTopBar], touch control while „Steuern“ is on and the screen
 * keyboard. [adopted] is the preview session of a tile of the group view: it is raised to the full
 * stream and goes back to the preview when this view closes.
 */
@Composable
fun ScreenViewScreen(target: ScreenTarget, title: String, onClose: () -> Unit, adopted: ScreenSession? = null) {
    val context = LocalContext.current
    val backend = LocalScreenBackend.current
    val scope = rememberCoroutineScope()
    val owned = remember { mutableListOf<ScreenSession>() }
    var current by remember { mutableStateOf(adopted) }
    var mode by remember { mutableStateOf(if (adopted != null) "preview" else if (target.control) "control" else "view") }
    var controlOn by remember { mutableStateOf(target.control) }
    var failure by remember { mutableStateOf<String?>(null) }
    var settings by remember { mutableStateOf(ViewSettings(source = target.source)) }
    var zoomMode by remember { mutableStateOf(false) }
    var keyboardOn by remember { mutableStateOf(false) }
    var barVisible by remember { mutableStateOf(true) }
    var surface by remember { mutableStateOf<Surface?>(null) }
    var bitmap by remember { mutableStateOf<Bitmap?>(null) }
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
            opened.open(newMode, settings.source, metrics.widthPixels, metrics.heightPixels, maxFps, backend.offers())
        } catch (e: CoreException) {
            failure = e.message
            return
        }
        val previous = current
        owned.add(opened)
        current = opened
        mode = newMode
        failure = null
        if (previous != null && previous !== adopted) {
            owned.remove(previous)
            previous.close()
        }
    }
    LaunchedEffect(Unit) {
        if (adopted == null) {
            openOwn(mode)
        } else {
            adopted.call { setMode(it, "full", settings.source, null, null, null, settings.sharpness, settings.lossless) }
            if (target.control) openOwn("control")
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
    val sending = controlOn && ui.canControl && mode == "control" && ui.holder != "other"
    fun releaseInputs() = sessionNow?.sendInput(mapperNow.releaseAll())
    LaunchedEffect(Unit) {
        for (signal in flush) {
            withFrameNanos { }
            sessionNow?.sendInput(mapperNow.drain())
        }
    }
    LaunchedEffect(sending, session?.id) {
        if (sending) session?.call { control(it, "take") } else releaseInputs()
    }
    LaunchedEffect(ui.ended) { if (ui.ended != null) releaseInputs() }
    LifecycleEventEffect(Lifecycle.Event.ON_PAUSE) { releaseInputs() }
    DisposableEffect(Unit) {
        onDispose {
            // First what is pressed, then the sessions (the queued input goes out before a session ends).
            releaseInputs()
            owned.forEach { it.close() }
            adopted?.call { setMode(it, "preview") }
        }
    }
    val view = LocalView.current
    DisposableEffect(view) {
        view.keepScreenOn = true
        onDispose { view.keepScreenOn = false }
    }
    DisposableEffect(session, surface) {
        val active = session
        if (active == null || active.id == 0L) return@DisposableEffect onDispose { }
        val decoder = ScreenDecoder(
            sessionId = active.id,
            source = backend.pictures(active.id),
            streamOf = { gop -> active.state.value.streams[gop] },
            surface = { surface },
            listener = object : ScreenDecoder.Listener {
                override fun onSize(width: Int, height: Int) = Unit
                override fun onBitmap(picture: Bitmap) {
                    bitmap = picture
                    pictures++
                }
                override fun onPicture() {
                    failures = 0
                }
                override fun onDecodeFailure(consecutive: Int) {
                    failures = consecutive
                    active.call { keyframe(it) }
                }
                override fun onEnded() = Unit
            },
        )
        decoder.start()
        // A decoder that starts in the middle of a picture group needs a key frame.
        active.call { keyframe(it) }
        onDispose { decoder.stop() }
    }

    fun applySettings(next: ViewSettings) {
        settings = next
        session?.call { setMode(it, "full", next.source, null, null, null, next.sharpness, next.lossless) }
    }
    val ended = ui.ended
    val status = when {
        ended != null -> ScreenTexts.endText(context, ended.code, ended.detail)
        failure != null -> failure.orEmpty()
        failures > 3 -> stringResource(R.string.screen_view_decode_failed)
        ui.blocked != null && sending -> ScreenTexts.blockedText(context, ui.blocked.orEmpty())
        ui.holder == "other" -> ScreenTexts.errorText(context, "control_busy")
        ui.errorCode != null -> ScreenTexts.errorText(context, ui.errorCode.orEmpty(), ui.errorDetail)
        else -> ScreenTexts.stateText(context, ui.state, ui.reason)
    }

    Dialog(
        onDismissRequest = onClose,
        properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false, dismissOnClickOutside = false),
    ) {
        val rootFocus = remember { FocusRequester() }
        Box(
            Modifier
                .fillMaxSize()
                .background(Color.Black)
                .focusRequester(rootFocus)
                .focusable()
                .onPreviewKeyEvent { event ->
                    val native = event.nativeKeyEvent
                    val known = ScreenInputMapper.keyName(native.keyCode) != null
                    if (sending && known && native.repeatCount == 0) {
                        mapperNow.onKey(native.keyCode, event.type == KeyEventType.KeyDown, native.metaState)
                        flush.trySend(Unit)
                    }
                    sending && known
                },
        ) {
            LaunchedEffect(sending) { if (sending) rootFocus.requestFocus() }
            BoxWithConstraints(Modifier.fillMaxSize()) {
                @Suppress("UNUSED_VARIABLE") val drawnFor = transformTick // zoom or pan changed: draw again
                val viewW = constraints.maxWidth.toFloat()
                val viewH = constraints.maxHeight.toFloat()
                geometry[0] = viewW
                geometry[1] = viewH
                geometry[2] = aspect
                val (w, h) = transform.contentSize(viewW, viewH, aspect)
                val left = (viewW - w) / 2 + transform.panX
                val top = (viewH - h) / 2 + transform.panY
                val density = LocalDensity.current
                val placed = Modifier
                    .offset { IntOffset(left.roundToInt(), top.roundToInt()) }
                    .size(with(density) { w.toDp() }, with(density) { h.toDp() })
                val shown = bitmap
                if (ui.stream?.codec == "tiles" && shown != null) {
                    key(pictures) { Image(bitmap = shown.asImageBitmap(), contentDescription = null, modifier = placed) }
                } else {
                    AndroidView(factory = { ctx -> surfaceView(ctx) { surface = it } }, modifier = placed)
                }
                val gestures = if (sending && !zoomMode) {
                    Modifier.mapperTouchInput(mapper, longPressMs) { flush.trySend(Unit) }
                } else {
                    Modifier
                        .pointerInput(Unit) { detectTapGestures(onTap = { barVisible = !barVisible }, onDoubleTap = { transform.reset() }) }
                        .pointerInput(Unit) {
                            detectTransformGestures { centroid, pan, zoom, _ ->
                                transform.zoomBy(zoom, centroid.x, centroid.y, viewW, viewH, aspect)
                                if (transform.zoom > 1f) transform.panBy(pan.x, pan.y, viewW, viewH, aspect)
                            }
                        }
                }
                Box(Modifier.fillMaxSize().then(gestures))
            }
            if (barVisible || sending) {
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
                        onAction = { mapper.action(it).also { flush.trySend(Unit) } },
                        onUsage = { mapper.tapUsage(it).also { flush.trySend(Unit) } },
                        onKeyboard = { keyboardOn = !keyboardOn },
                        onZoomMode = { zoomMode = !zoomMode },
                        onSettings = { applySettings(it) },
                        onStatus = if (failures > 3 && !settings.lossless) ({ failures = 0; applySettings(settings.copy(lossless = true)) }) else null,
                        onClose = onClose,
                    )
                }
            }
            if (ended != null) {
                Row(Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(16.dp)) {
                    TextButton(onClick = { scope.launch { openOwn(if (controlOn) "control" else "view") } }) {
                        Text(stringResource(R.string.screen_view_reconnect), color = Color.White)
                    }
                    TextButton(onClick = onClose) { Text(stringResource(R.string.screen_view_close), color = Color.White) }
                }
            }
            if (sending && keyboardOn) ScreenKeyboardField(mapper, flush)
        }
    }
}

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
    val pad = "​".repeat(FIELD_PAD)
    var text by remember { mutableStateOf(pad) }
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(Unit) {
        focus.requestFocus()
        keyboard?.show()
    }
    DisposableEffect(Unit) { onDispose { keyboard?.hide() } }
    BasicTextField(
        value = text,
        onValueChange = { next ->
            when {
                next.length > text.length && next.startsWith(text) -> mapper.onText(next.substring(text.length))
                next.length < text.length -> repeat(text.length - next.length) { mapper.tapKey("backspace") }
            }
            flush.trySend(Unit)
            text = if (next.length < FIELD_PAD / 2 || next.length > FIELD_PAD * 8) pad else next
        },
        modifier = Modifier.size(1.dp).alpha(0f).focusRequester(focus),
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
        keyboardActions = KeyboardActions(onSend = {
            mapper.tapKey("enter")
            flush.trySend(Unit)
        }),
    )
}
