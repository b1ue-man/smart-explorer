package app.smartexplorer.android.ui.viewer

import android.media.AudioManager
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.annotation.DrawableRes
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.platform.LocalContext
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.smartexplorer.android.R
import app.smartexplorer.android.ui.common.SeIcon
import kotlinx.coroutines.delay

private const val POSITION_POLL_MS = 250L

/**
 * One video or audio file. Only the [active] (settled) page holds a platform player
 * (`MediaPlayer` with a replaceable video surface); it restores position and playback intent once prepared and is released when another page is
 * shown. Neighbor pages show [name] and the media symbol. The controls are Compose elements
 * above the video, so one-finger swipes still page; a tap calls [onTap].
 */
@Composable
internal fun PlayerPage(
    media: LocalMedia,
    name: String,
    audio: Boolean,
    active: Boolean,
    controlsVisible: Boolean,
    onTap: () -> Unit,
    failed: @Composable (String) -> Unit,
) {
    var positionMs by rememberSaveable(media.path) { mutableIntStateOf(0) }
    var wantsPlayback by rememberSaveable(media.path) { mutableStateOf(true) }
    if (!active) {
        MediaTitle(name, if (audio) R.drawable.ic_audio else R.drawable.ic_video, onTap)
        return
    }
    var prepared by remember(media.path) { mutableStateOf(false) }
    var playing by remember(media.path) { mutableStateOf(false) }
    var error by remember(media.path) { mutableStateOf<String?>(null) }
    var durationMs by remember(media.path) { mutableIntStateOf(0) }
    var videoWidth by remember(media.path) { mutableIntStateOf(0) }
    var videoHeight by remember(media.path) { mutableIntStateOf(0) }
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    var playbackState by remember(media.path) { mutableStateOf<MediaPlaybackState?>(null) }
    DisposableEffect(media.path, lifecycleOwner) {
        // Allocate after the previous page's effect has released its player.
        val playback = MediaPlaybackState(context.applicationContext)
        val player = playback.player
        prepared = false
        playing = false
        error = null
        playbackState = playback
        val lifecycle = lifecycleOwner.lifecycle
        fun visible() = lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_STOP -> {
                    // Returning to the viewer stays paused until an explicit Play.
                    wantsPlayback = false
                    playing = false
                    if (prepared && error == null) positionMs = player.currentPosition.coerceAtLeast(0)
                    playback.pause(prepared && error == null)
                }
                Lifecycle.Event.ON_START -> {
                    // Initial preparation can finish before the UI reaches STARTED.
                    if (wantsPlayback && prepared && error == null) playing = playback.start(visible())
                }
                else -> Unit
            }
        }
        lifecycle.addObserver(observer)
        playback.onFocusChange = { change ->
            if (prepared && error == null) {
                when (change) {
                    AudioManager.AUDIOFOCUS_LOSS, AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> {
                        playback.pause()
                        playing = false
                        // Focus loss requires an explicit resume, never override a user's pause.
                        wantsPlayback = false
                        positionMs = player.currentPosition.coerceAtLeast(0)
                    }
                    AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> player.setVolume(0.2f, 0.2f)
                    AudioManager.AUDIOFOCUS_GAIN -> player.setVolume(1f, 1f)
                }
            }
        }
        player.setOnVideoSizeChangedListener { _, width, height -> videoWidth = width; videoHeight = height }
        player.setOnPreparedListener {
            durationMs = it.duration.coerceAtLeast(0)
            videoWidth = it.videoWidth
            videoHeight = it.videoHeight
            prepared = true
            it.seekTo(positionMs)
            if (wantsPlayback) playing = playback.start(visible())
        }
        player.setOnCompletionListener {
            playing = false
            wantsPlayback = false
            playback.pause()
            positionMs = durationMs
        }
        player.setOnErrorListener { _, what, extra ->
            error = "Wiedergabe nicht möglich (Fehler $what/$extra)"
            playing = false
            true
        }
        try {
            player.setDataSource(media.path)
            player.prepareAsync()
        } catch (failure: Exception) {
            error = "Wiedergabe nicht möglich: ${failure.message ?: "Datei nicht lesbar"}"
        }
        onDispose {
            lifecycle.removeObserver(observer)
            if (prepared && error == null) positionMs = player.currentPosition.coerceAtLeast(0)
            playback.release()
        }
    }
    val playback = playbackState ?: return
    val player = playback.player
    LaunchedEffect(playing) {
        while (playing) {
            if (prepared && error == null) positionMs = player.currentPosition.coerceAtLeast(0)
            delay(POSITION_POLL_MS)
        }
    }
    error?.let {
        LaunchedEffect(playback) { playback.release() }
        failed(it)
        return
    }
    BoxWithConstraints(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        val wide = maxWidth > maxHeight
        if (!audio) {
            val ratio = if (videoWidth > 0 && videoHeight > 0) videoWidth.toFloat() / videoHeight else 1f
            val fittedWidth = minOf(maxWidth, maxHeight * ratio)
            val fittedHeight = fittedWidth / ratio
            AndroidView(
                factory = { context ->
                    SurfaceView(context).apply {
                        holder.addCallback(object : SurfaceHolder.Callback {
                            override fun surfaceCreated(holder: SurfaceHolder) { playback.display(holder) }
                            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) = Unit
                            override fun surfaceDestroyed(holder: SurfaceHolder) { playback.display(null) }
                        })
                    }
                },
                modifier = Modifier.widthIn(max = fittedWidth).height(fittedHeight).fillMaxWidth(),
                onRelease = { playback.display(null) },
            )
        }
        // Tap target above the video: taps toggle the bars, drags reach the pager.
        if (audio) {
            MediaTitle(name, R.drawable.ic_audio, onTap, wide)
        } else {
            Box(Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures(onTap = { onTap() }) })
        }
        if (controlsVisible || !playing) {
            Controls(
                playing = playing,
                enabled = prepared,
                positionMs = positionMs,
                durationMs = durationMs,
                onToggle = {
                    val view = player
                    if (view.isPlaying) {
                        playback.pause()
                        playing = false
                        wantsPlayback = false
                        positionMs = view.currentPosition.coerceAtLeast(0)
                    } else {
                        if (durationMs > 0 && positionMs >= durationMs) view.seekTo(0)
                        playing = playback.start(lifecycleOwner.lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
                        wantsPlayback = playing
                    }
                },
                onSeek = { target ->
                    positionMs = target
                    player.seekTo(target)
                },
                modifier = Modifier.align(Alignment.BottomCenter),
            )
        }
    }
}

/** Symbol and name of a medium without a picture (audio, or a video that is not shown yet). */
@Composable
private fun MediaTitle(name: String, @DrawableRes icon: Int, onTap: () -> Unit, wide: Boolean = false) {
    val tap = Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures(onTap = { onTap() }) }
    val title: @Composable () -> Unit = {
        Text(
            name,
            color = Color.White,
            style = MaterialTheme.typography.titleMedium,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(16.dp),
            maxLines = if (wide) 2 else 4,
            overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
        )
    }
    if (wide) {
        Row(
            tap.padding(horizontal = 48.dp, vertical = 64.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center,
        ) {
            SeIcon(icon, null, Modifier.size(48.dp), tint = Color.White)
            Box(Modifier.weight(1f, fill = false)) { title() }
        }
    } else {
        Column(tap, horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
            SeIcon(icon, null, Modifier.size(96.dp), tint = Color.White)
            title()
        }
    }
}

@Composable
private fun Controls(
    playing: Boolean,
    enabled: Boolean,
    positionMs: Int,
    durationMs: Int,
    onToggle: () -> Unit,
    onSeek: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    BoxWithConstraints(modifier.fillMaxWidth().background(Color.Black.copy(alpha = 0.55f))) {
        val compact = maxWidth < 360.dp
        Column(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp)) {
            if (compact) {
                Text("${clock(positionMs)} / ${clock(durationMs)}", color = Color.White,
                    style = MaterialTheme.typography.labelMedium, modifier = Modifier.align(Alignment.End))
            }
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                IconButton(onClick = onToggle, enabled = enabled) {
                    SeIcon(if (playing) R.drawable.ic_pause else R.drawable.ic_play,
                        if (playing) "Pause" else "Abspielen", tint = Color.White)
                }
                Slider(
                    value = if (durationMs > 0) positionMs.coerceIn(0, durationMs).toFloat() else 0f,
                    onValueChange = { onSeek(it.toInt()) },
                    valueRange = 0f..durationMs.coerceAtLeast(1).toFloat(),
                    enabled = enabled && durationMs > 0,
                    modifier = Modifier.weight(1f),
                )
                if (!compact) {
                    Text("${clock(positionMs)} / ${clock(durationMs)}", color = Color.White,
                        style = MaterialTheme.typography.labelMedium)
                }
            }
        }
    }
}
