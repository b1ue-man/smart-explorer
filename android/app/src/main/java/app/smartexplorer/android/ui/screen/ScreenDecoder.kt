package app.smartexplorer.android.ui.screen

import android.graphics.Bitmap
import android.media.MediaCodec
import android.media.MediaCodecInfo.CodecProfileLevel
import android.media.MediaCodecList
import android.media.MediaFormat
import android.util.Log
import android.view.Surface
import app.smartexplorer.android.api.DecoderOffer
import app.smartexplorer.android.core.NativeBridge
import app.smartexplorer.android.core.ScreenViewEvent
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Where the pictures come from: `NativeBridge.screenPull` of a session, a fake in tests. */
fun interface PictureSource {
    fun pull(dst: ByteBuffer, timeoutMs: Int): Int
}

/** The pictures of viewer session [session] through the JNI bridge. */
class NativePictureSource(private val session: Long) : PictureSource {
    override fun pull(dst: ByteBuffer, timeoutMs: Int): Int = NativeBridge.screenPull(session, dst, timeoutMs)
}

/** What one decoder of the device can do (a `MediaCodecInfo`, reduced to what the offer needs). */
data class DecoderCaps(val hardware: Boolean, val avcProfiles: Set<Int>, val maxWidth: Int, val maxHeight: Int)

/**
 * Decodes the pictures of a viewer session (SCR-M9, U28-5) on the thread `screen-decode-<session>`.
 * It pulls into the input buffer of a MediaCodec decoder (a pull is 16 bytes of head, then the
 * payload: `viewer_sink.rs`), configures it at the first picture of a picture group (flag
 * `NEW_STREAM`) with `KEY_LOW_LATENCY` and renders every output at once onto the surface; finished
 * tile pictures (RGBA) become a bitmap. Nothing of a picture is logged or kept beyond the last one.
 */
class ScreenDecoder(
    private val sessionId: Long,
    private val source: PictureSource,
    /** The group's `stream` event for the low 16 bits of its number, once it arrived. */
    private val streamOf: (Int) -> ScreenViewEvent.Stream?,
    private val surface: () -> Surface?,
    private val listener: Listener,
) {
    interface Listener {
        fun onSize(width: Int, height: Int)
        fun onBitmap(bitmap: Bitmap)
        fun onPicture()
        /** The decoder failed [consecutive] times in a row without a picture in between. */
        fun onDecodeFailure(consecutive: Int)
        fun onEnded()
    }

    @Volatile private var running = false
    private var thread: Thread? = null
    private var codec: MediaCodec? = null
    private var codecGop = -1
    private var heldIndex = -1
    private var staging = ByteBuffer.allocateDirect(STAGING_START)
    private var bitmap: Bitmap? = null
    private var failures = 0

    fun start() {
        running = true
        thread = Thread({ run() }, "screen-decode-$sessionId").also { it.start() }
    }

    /** Asks the thread to end; it does within one pull. */
    fun stop() {
        running = false
    }

    private fun run() {
        try {
            loop()
        } catch (e: RuntimeException) {
            Log.w(TAG, "decode loop ended: ${e.javaClass.simpleName}")
        } finally {
            releaseCodec()
            listener.onEnded()
        }
    }

    private fun loop() {
        while (running) {
            val active = codec
            var dst = staging
            if (active != null) {
                drain(active)
                if (codec !== active) continue
                if (heldIndex < 0) heldIndex = try { active.dequeueInputBuffer(INPUT_WAIT_US) } catch (e: IllegalStateException) { fail(); continue }
                val buffer = if (heldIndex >= 0) active.getInputBuffer(heldIndex) else null
                if (buffer == null) {
                    heldIndex = -1
                    continue
                }
                dst = buffer
            }
            dst.clear()
            dst.order(ByteOrder.LITTLE_ENDIAN)
            val n = source.pull(dst, PULL_WAIT_MS)
            when {
                n > HEADER_BYTES -> accept(dst, n)
                n == PULL_TIMEOUT -> Unit
                n == PULL_TOO_SMALL -> pullLarger(dst.getInt(12))
                else -> return
            }
        }
    }

    /** A picture that did not fit the input buffer: pulled into a larger direct buffer, then copied. */
    private fun pullLarger(payloadLen: Int) {
        if (payloadLen <= 0) return
        staging = ByteBuffer.allocateDirect(payloadLen + HEADER_BYTES).order(ByteOrder.LITTLE_ENDIAN)
        val n = source.pull(staging, PULL_WAIT_MS)
        if (n > HEADER_BYTES) accept(staging, n)
    }

    private fun accept(src: ByteBuffer, n: Int) {
        val pts = src.getLong(0)
        val flags = src.getInt(8)
        val len = src.getInt(12)
        if (len != n - HEADER_BYTES) return
        val gop = flags ushr GOP_SHIFT
        if (flags and FLAG_TILES != 0) {
            drawBitmap(src, gop, len)
            return
        }
        val keyframe = flags and FLAG_KEYFRAME != 0
        val newStream = flags and FLAG_NEW_STREAM != 0
        if (codec != null && !newStream) {
            queue(src, pts, keyframe, len)
            return
        }
        if (!newStream && !keyframe) return
        // A new group (or a decoder that failed): the old decoder goes, its buffers with it.
        val payload = ByteArray(len)
        src.position(HEADER_BYTES)
        src.get(payload)
        val info = awaitStream(gop) ?: return
        releaseCodec()
        if (!configure(info, gop)) return
        queueBytes(payload, pts, keyframe)
    }

    private fun queue(src: ByteBuffer, pts: Long, keyframe: Boolean, len: Int) {
        val c = codec ?: return
        val key = if (keyframe) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0
        try {
            if (src === staging) {
                val payload = ByteArray(len)
                src.position(HEADER_BYTES)
                src.get(payload)
                queueBytes(payload, pts, keyframe)
            } else {
                c.queueInputBuffer(heldIndex, HEADER_BYTES, len, pts, key)
                heldIndex = -1
            }
        } catch (e: MediaCodec.CodecException) {
            fail()
        } catch (e: IllegalStateException) {
            fail()
        }
    }

    /** Copies [payload] into the next free input buffer of the decoder. */
    private fun queueBytes(payload: ByteArray, pts: Long, keyframe: Boolean) {
        val c = codec ?: return
        try {
            var index = heldIndex
            var tries = 0
            while (index < 0 && running && tries++ < INPUT_TRIES) index = c.dequeueInputBuffer(INPUT_WAIT_US)
            heldIndex = -1
            val buffer = if (index >= 0) c.getInputBuffer(index) else null
            if (buffer == null || buffer.capacity() < payload.size) {
                // Too large for the decoder's input (an empty buffer cannot be given back): start over.
                fail()
                return
            }
            buffer.clear()
            buffer.put(payload)
            c.queueInputBuffer(index, 0, payload.size, pts, if (keyframe) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0)
        } catch (e: MediaCodec.CodecException) {
            fail()
        } catch (e: IllegalStateException) {
            fail()
        }
    }

    private fun awaitStream(gop: Int): ScreenViewEvent.Stream? {
        var waited = 0
        while (running && waited <= STREAM_WAIT_MS) {
            streamOf(gop)?.let { return it }
            Thread.sleep(STREAM_POLL_MS)
            waited += STREAM_POLL_MS.toInt()
        }
        return null
    }

    private fun configure(info: ScreenViewEvent.Stream, gop: Int): Boolean {
        val target = surface() ?: return false
        val format = MediaFormat.createVideoFormat(MIME, info.width, info.height)
        // Both keys are hints that older decoders ignore.
        try { format.setInteger(MediaFormat.KEY_LOW_LATENCY, 1) } catch (e: IllegalArgumentException) { Unit }
        try { format.setInteger(MediaFormat.KEY_PRIORITY, 0) } catch (e: IllegalArgumentException) { Unit }
        format.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, info.width * info.height * 3)
        return try {
            val created = MediaCodec.createDecoderByType(MIME)
            try {
                created.configure(format, target, null, 0)
                created.start()
            } catch (e: Exception) {
                created.release()
                throw e
            }
            codec = created
            codecGop = gop
            true
        } catch (e: Exception) {
            Log.w(TAG, "decoder not started: ${e.javaClass.simpleName}")
            fail()
            false
        }
    }

    private fun drain(c: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        try {
            while (true) {
                val index = c.dequeueOutputBuffer(info, 0)
                when {
                    index >= 0 -> {
                        c.releaseOutputBuffer(index, true)
                        failures = 0
                        listener.onPicture()
                    }
                    index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED ->
                        c.outputFormat.let { listener.onSize(it.getInteger(MediaFormat.KEY_WIDTH), it.getInteger(MediaFormat.KEY_HEIGHT)) }
                    else -> return
                }
            }
        } catch (e: MediaCodec.CodecException) {
            fail()
        } catch (e: IllegalStateException) {
            fail()
        }
    }

    private fun drawBitmap(src: ByteBuffer, gop: Int, len: Int) {
        val info = awaitStream(gop) ?: return
        if (len != info.width * info.height * 4) return
        var target = bitmap
        if (target == null || target.width != info.width || target.height != info.height) {
            target = Bitmap.createBitmap(info.width, info.height, Bitmap.Config.ARGB_8888)
            bitmap = target
        }
        src.position(HEADER_BYTES)
        src.limit(HEADER_BYTES + len)
        target.copyPixelsFromBuffer(src)
        src.clear()
        listener.onBitmap(target)
    }

    private fun fail() {
        failures++
        releaseCodec()
        listener.onDecodeFailure(failures)
    }

    private fun releaseCodec() {
        heldIndex = -1
        codecGop = -1
        val old = codec ?: return
        codec = null
        try {
            old.stop()
        } catch (e: IllegalStateException) {
            Unit
        }
        old.release()
    }

    companion object {
        private const val TAG = "ScreenDecoder"
        private const val MIME = "video/avc"
        const val HEADER_BYTES = 16
        const val FLAG_KEYFRAME = 1
        const val FLAG_TILES = 2
        const val FLAG_NEW_STREAM = 4
        const val GOP_SHIFT = 16
        const val PULL_TIMEOUT = -1
        const val PULL_ENDED = -2
        const val PULL_TOO_SMALL = -3
        private const val PULL_WAIT_MS = 100
        /** The decoding thread looks at its stop flag at least this often (10 ms). */
        private const val INPUT_WAIT_US = 10_000L
        private const val INPUT_TRIES = 100
        /** The `stream` event arrives through the event poll, a moment after the first picture. */
        private const val STREAM_WAIT_MS = 2_000
        private const val STREAM_POLL_MS = 20L
        private const val STAGING_START = 1 shl 20

        /** The decoders this phone offers the host: H.264 per profile and hardware, and always `tiles`. */
        fun offers(): List<DecoderOffer> {
            val caps = try {
                MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
                    .filter { !it.isEncoder && MIME in it.supportedTypes }
                    .map { info ->
                        val c = info.getCapabilitiesForType(MIME)
                        val video = c.videoCapabilities
                        DecoderCaps(
                            hardware = info.isHardwareAccelerated,
                            avcProfiles = c.profileLevels.map { it.profile }.toSet(),
                            maxWidth = video?.supportedWidths?.upper ?: 0,
                            maxHeight = video?.supportedHeights?.upper ?: 0,
                        )
                    }
            } catch (e: RuntimeException) {
                emptyList()
            }
            return offersFor(caps)
        }

        /** [offers] for the given decoders. */
        fun offersFor(codecs: List<DecoderCaps>): List<DecoderOffer> {
            val wanted = listOf(
                "high" to setOf(CodecProfileLevel.AVCProfileHigh),
                "cb" to setOf(CodecProfileLevel.AVCProfileConstrainedBaseline, CodecProfileLevel.AVCProfileBaseline),
            )
            val h264 = wanted.flatMap { (profile, accepted) ->
                listOf(true, false).mapNotNull { hardware ->
                    val fits = codecs.filter { it.hardware == hardware && it.avcProfiles.any { p -> p in accepted } }
                    if (fits.isEmpty()) {
                        null
                    } else {
                        DecoderOffer(
                            codec = "h264",
                            profile = profile,
                            hardware = hardware,
                            maxWidth = fits.maxOf { it.maxWidth }.takeIf { it > 0 },
                            maxHeight = fits.maxOf { it.maxHeight }.takeIf { it > 0 },
                        )
                    }
                }
            }
            return h264 + DecoderOffer(codec = "tiles")
        }
    }
}
