package app.smartexplorer.android.ui.viewer

import android.widget.VideoView
import androidx.annotation.DrawableRes
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import app.smartexplorer.android.R
import app.smartexplorer.android.ui.common.SeIcon
import kotlinx.coroutines.delay

private const val POSITION_POLL_MS = 250L

/**
 * One video or audio file. Only the [active] (settled) page holds a platform player
 * (`VideoView` → `MediaPlayer`); it starts once prepared and is released when another page is
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
    if (!active) {
        MediaTitle(name, if (audio) R.drawable.ic_audio else R.drawable.ic_video, onTap)
        return
    }
    var player by remember(media.path) { mutableStateOf<VideoView?>(null) }
    var prepared by remember(media.path) { mutableStateOf(false) }
    var playing by remember(media.path) { mutableStateOf(false) }
    var error by remember(media.path) { mutableStateOf<String?>(null) }
    var durationMs by remember(media.path) { mutableIntStateOf(0) }
    var positionMs by remember(media.path) { mutableIntStateOf(0) }

    LaunchedEffect(playing) {
        while (playing) {
            player?.let { positionMs = it.currentPosition }
            delay(POSITION_POLL_MS)
        }
    }

    error?.let {
        failed(it)
        return
    }
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        AndroidView(
            factory = { context ->
                VideoView(context).apply {
                    isFocusable = false
                    isClickable = false
                    setOnPreparedListener { mediaPlayer ->
                        durationMs = mediaPlayer.duration.coerceAtLeast(0)
                        prepared = true
                        start()
                        playing = true
                    }
                    setOnCompletionListener {
                        playing = false
                        positionMs = durationMs
                    }
                    setOnErrorListener { _, what, extra ->
                        error = "Wiedergabe nicht möglich (Fehler $what/$extra)"
                        playing = false
                        true
                    }
                    setVideoPath(media.path)
                    player = this
                }
            },
            onRelease = { it.stopPlayback() },
        )
        // Tap target above the video: taps toggle the bars, drags reach the pager.
        if (audio) {
            MediaTitle(name, R.drawable.ic_audio, onTap)
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
                    val view = player ?: return@Controls
                    if (view.isPlaying) {
                        view.pause()
                        playing = false
                    } else {
                        if (durationMs > 0 && positionMs >= durationMs) view.seekTo(0)
                        view.start()
                        playing = true
                    }
                },
                onSeek = { target ->
                    positionMs = target
                    player?.seekTo(target)
                },
                modifier = Modifier.align(Alignment.BottomCenter),
            )
        }
    }
}

/** Symbol and name of a medium without a picture (audio, or a video that is not shown yet). */
@Composable
private fun MediaTitle(name: String, @DrawableRes icon: Int, onTap: () -> Unit) {
    Column(
        Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures(onTap = { onTap() }) },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        SeIcon(icon, null, Modifier.size(96.dp), tint = Color.White)
        Text(
            name,
            color = Color.White,
            style = MaterialTheme.typography.titleMedium,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(16.dp),
        )
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
    Row(
        modifier
            .fillMaxWidth()
            .background(Color.Black.copy(alpha = 0.55f))
            .padding(horizontal = 8.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        IconButton(onClick = onToggle, enabled = enabled) {
            SeIcon(
                if (playing) R.drawable.ic_pause else R.drawable.ic_play,
                if (playing) "Pause" else "Abspielen",
                tint = Color.White,
            )
        }
        Slider(
            value = if (durationMs > 0) positionMs.coerceIn(0, durationMs).toFloat() else 0f,
            onValueChange = { onSeek(it.toInt()) },
            valueRange = 0f..durationMs.coerceAtLeast(1).toFloat(),
            enabled = enabled && durationMs > 0,
            modifier = Modifier.weight(1f),
        )
        Text(
            "${clock(positionMs)} / ${clock(durationMs)}",
            color = Color.White,
            style = MaterialTheme.typography.labelMedium,
        )
    }
}

/** `m:ss` or `h:mm:ss`. */
internal fun clock(ms: Int): String {
    val total = (ms.coerceAtLeast(0) / 1000)
    val hours = total / 3600
    val minutes = (total % 3600) / 60
    val seconds = total % 60
    return if (hours > 0) {
        "%d:%02d:%02d".format(hours, minutes, seconds)
    } else {
        "%d:%02d".format(minutes, seconds)
    }
}
