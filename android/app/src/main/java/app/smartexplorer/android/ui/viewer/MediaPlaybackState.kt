package app.smartexplorer.android.ui.viewer

import android.media.MediaPlayer
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.content.Context
import android.view.SurfaceHolder

/** The surface can disappear before or after Compose disposes the player. */
internal class MediaPlaybackState(context: Context) {
    private val attributes = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build()
    val player = MediaPlayer().apply { setAudioAttributes(attributes) }
    private val audioManager = context.getSystemService(AudioManager::class.java)
    var onFocusChange: (Int) -> Unit = {}
    private val focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
        .setAudioAttributes(attributes).setOnAudioFocusChangeListener { onFocusChange(it) }.build()
    // Prepared is not a valid pause state. isPlaying() can lag behind start(),
    // so retain our successful start transition until pause or release.
    private var started = false
    fun start(visible: Boolean): Boolean {
        val manager = audioManager ?: return false
        if (released || !visible || manager.requestAudioFocus(focus) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED) return false
        player.start()
        started = true
        return true
    }
    fun pause(prepared: Boolean = true) {
        if (!released && prepared && started) {
            player.pause()
            started = false
        }
        audioManager?.abandonAudioFocusRequest(focus)
    }
    private var released = false
    private var displayHolder: SurfaceHolder? = null

    fun display(holder: SurfaceHolder?) {
        if (released) return
        // MediaPlayer updates only its current holder. Clear the old holder's
        // wake request before setDisplay replaces it, including on release.
        player.setScreenOnWhilePlaying(false)
        player.setDisplay(holder)
        displayHolder = holder
        player.setScreenOnWhilePlaying(holder != null)
    }

    fun clearDisplay(holder: SurfaceHolder) {
        // A delayed callback from a replaced SurfaceView must not detach its successor.
        if (displayHolder === holder) display(null)
    }

    fun release() {
        if (released) return
        display(null)
        released = true
        started = false
        audioManager?.abandonAudioFocusRequest(focus)
        onFocusChange = {}
        player.setOnPreparedListener(null)
        player.setOnCompletionListener(null)
        player.setOnErrorListener(null)
        player.setOnVideoSizeChangedListener(null)
        player.release()
    }
}
