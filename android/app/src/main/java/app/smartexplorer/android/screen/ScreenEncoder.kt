package app.smartexplorer.android.screen

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Bundle
import android.util.Log
import android.view.Surface
import app.smartexplorer.android.api.ScreenEncoderOffer
import java.nio.ByteBuffer
import kotlin.concurrent.thread

/** Where encoded pictures go: the core (`NativeBridge.screenPush`) or a test. */
internal fun interface PictureSink {
    /** One access unit in Annex B form, `length` bytes of [data] from `offset`; returns the push code. */
    fun push(data: ByteBuffer, offset: Int, length: Int, ptsUs: Long, keyframe: Boolean): Int
}

/** What `configure` of the core asks for (C2: one encoder per picture group of the pipeline). */
internal data class EncoderConfig(
    val epoch: Int,
    val width: Int,
    val height: Int,
    val fps: Int,
    val bitrate: Int,
    /** `cb` or `high`. */
    val profile: String,
)

/**
 * H.264 of this phone as screen host (C2 Android): the MediaCodec encoder with an input surface
 * that the virtual display of MediaProjection or the test picture draws into. Hardware encoders
 * first (`isHardwareAccelerated`), then the system's software encoder. Every key frame carries
 * SPS and PPS in front, so each picture group starts decodable; key frames come when the core asks
 * (`requestKeyFrame`) and at most every [KEYFRAME_INTERVAL_S]. Nothing of a picture is logged.
 */
internal class ScreenEncoder(private val config: EncoderConfig, private val sink: PictureSink) {
    private val codec: MediaCodec
    val surface: Surface
    val name: String
    val hardware: Boolean

    @Volatile
    private var running = true
    private var codecConfig: ByteArray? = null
    private var joined: ByteBuffer = ByteBuffer.allocateDirect(0)
    private val drain: Thread

    init {
        val info = pick(config.profile) ?: throw IllegalStateException("Kein H.264-Kodierer auf diesem Telefon")
        name = info.name
        hardware = info.isHardwareAccelerated
        codec = MediaCodec.createByCodecName(info.name)
        val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, config.width, config.height).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, config.bitrate.coerceAtLeast(MIN_BITRATE))
            setInteger(MediaFormat.KEY_FRAME_RATE, config.fps.coerceIn(1, 60))
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, KEYFRAME_INTERVAL_S)
            setInteger(MediaFormat.KEY_PROFILE, profileOf(config.profile))
            setInteger(MediaFormat.KEY_PRIORITY, 0)
            // A still screen gives the surface no new picture: repeating the last one keeps a
            // requested key frame from waiting for a change on screen.
            setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, REPEAT_AFTER_US)
        }
        try {
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        } catch (e: RuntimeException) {
            // Some encoders refuse the profile key: the default profile then.
            format.removeKey(MediaFormat.KEY_PROFILE)
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        }
        surface = codec.createInputSurface()
        codec.start()
        drain = thread(name = "screen-encoder", isDaemon = true) { drainLoop() }
    }

    fun requestKeyFrame() = parameters { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) }

    fun setBitrate(bps: Int) = parameters { putInt(MediaCodec.PARAMETER_KEY_VIDEO_BITRATE, bps.coerceAtLeast(MIN_BITRATE)) }

    private fun parameters(fill: Bundle.() -> Unit) {
        try {
            codec.setParameters(Bundle().apply(fill))
        } catch (e: IllegalStateException) {
            Log.w(TAG, "Kodiererparameter nicht gesetzt: ${e.message}")
        }
    }

    fun release() {
        running = false
        try {
            codec.signalEndOfInputStream()
        } catch (e: IllegalStateException) {
            // Already stopped.
        }
        drain.join(DRAIN_JOIN_MS)
        try {
            codec.stop()
        } catch (e: IllegalStateException) {
            // Already stopped.
        }
        codec.release()
        surface.release()
    }

    private fun drainLoop() {
        val info = MediaCodec.BufferInfo()
        while (running) {
            val index = try {
                codec.dequeueOutputBuffer(info, DEQUEUE_TIMEOUT_US)
            } catch (e: IllegalStateException) {
                return
            }
            if (index < 0) continue
            try {
                val buffer = codec.getOutputBuffer(index) ?: continue
                deliver(buffer, info)
            } finally {
                codec.releaseOutputBuffer(index, false)
            }
            if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) return
        }
    }

    private fun deliver(buffer: ByteBuffer, info: MediaCodec.BufferInfo) {
        if (info.size <= 0) return
        if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
            codecConfig = ByteArray(info.size).also {
                buffer.position(info.offset)
                buffer.get(it)
            }
            return
        }
        val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
        val config = codecConfig
        if (!key || config == null) {
            sink.push(buffer, info.offset, info.size, info.presentationTimeUs, key)
            return
        }
        val total = config.size + info.size
        if (joined.capacity() < total) joined = ByteBuffer.allocateDirect(total)
        joined.clear()
        joined.put(config)
        buffer.limit(info.offset + info.size)
        buffer.position(info.offset)
        joined.put(buffer)
        sink.push(joined, 0, total, info.presentationTimeUs, true)
    }

    companion object {
        private const val TAG = "SmartExplorerScreen"
        const val KEYFRAME_INTERVAL_S = 10
        private const val MIN_BITRATE = 100_000
        private const val DEQUEUE_TIMEOUT_US = 100_000L
        private const val DRAIN_JOIN_MS = 1_000L
        private const val REPEAT_AFTER_US = 500_000L

        private fun profileOf(key: String): Int = when (key) {
            "cb" -> MediaCodecInfo.CodecProfileLevel.AVCProfileConstrainedBaseline
            else -> MediaCodecInfo.CodecProfileLevel.AVCProfileHigh
        }

        private fun encoders(): List<MediaCodecInfo> = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
            .filter { it.isEncoder && MediaFormat.MIMETYPE_VIDEO_AVC in it.supportedTypes.map(String::lowercase) && !it.isAlias }
            .sortedByDescending { it.isHardwareAccelerated }

        private fun supports(info: MediaCodecInfo, profile: String): Boolean {
            val levels = info.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC).profileLevels
            return when (profile) {
                "cb" -> levels.any {
                    it.profile == MediaCodecInfo.CodecProfileLevel.AVCProfileConstrainedBaseline ||
                        it.profile == MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline
                }
                else -> levels.any { it.profile == MediaCodecInfo.CodecProfileLevel.AVCProfileHigh }
            }
        }

        private fun pick(profile: String): MediaCodecInfo? =
            encoders().firstOrNull { supports(it, profile) } ?: encoders().firstOrNull()

        /** Name and hardware flag of the encoder [profile] would take, `null` without one. */
        fun best(profile: String = "high"): Pair<String, Boolean>? = pick(profile)?.let { it.name to it.isHardwareAccelerated }

        /** The offers this phone reports to the core (`screen.hostPrereqs`). */
        fun offers(): List<ScreenEncoderOffer> = buildList {
            for (profile in listOf("high", "cb")) {
                encoders().firstOrNull { supports(it, profile) }?.let { add(ScreenEncoderOffer(profile, it.isHardwareAccelerated)) }
            }
        }
    }
}
