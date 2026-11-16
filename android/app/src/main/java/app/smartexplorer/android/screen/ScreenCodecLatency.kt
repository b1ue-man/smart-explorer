package app.smartexplorer.android.screen

import android.media.MediaCodec
import java.nio.ByteBuffer
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Build

/** Admission for the existing serial hybrid transaction; never a device benchmark. */
internal object ScreenCodecLatency {
    // The existing host key-frame / viewer reference transaction deadline.
    const val TRANSACTION_NS = 2_000_000_000L
    const val POLL_US = 10_000L

    class Unavailable(codec: String, detail: String) : IllegalStateException(
        "Codec-Fallback (${codec.take(256)}): $detail"
    )

    /** Constants of the phone's encoder and decoder hints (B-26, B-39); the API levels are those of the Android reference. */
    const val NO_PERIODIC_KEYFRAMES = -1
    /** `KEY_INTRA_REFRESH_PERIOD` and `CodecCapabilities.FEATURE_IntraRefresh`: API 24. */
    const val SDK_INTRA_REFRESH = 24
    /** `KEY_LATENCY` (encoder): API 26. The app's minSdk is 30, so this check is true on every device that runs the app. */
    const val SDK_ENCODER_LATENCY = 26
    /** `KEY_LOW_LATENCY` and `CodecCapabilities.FEATURE_LowLatency` (decoder): API 30. */
    const val SDK_DECODER_LATENCY = 30

    /** One attempt of the encoder configuration. MODERN adds the new hints; the other two are today's settings. */
    enum class EncoderHints { MODERN, TODAY_REALTIME, TODAY_PLAIN }

    /**
     * The attempts in their order; a refused attempt moves on to the next one without ending the session.
     * Hybrid pictures (one input per access unit) keep today's attempts only.
     */
    fun encoderHintTiers(hybrid: Boolean): List<EncoderHints> =
        if (hybrid) listOf(EncoderHints.TODAY_REALTIME, EncoderHints.TODAY_PLAIN)
        else listOf(EncoderHints.MODERN, EncoderHints.TODAY_REALTIME, EncoderHints.TODAY_PLAIN)

    /** `KEY_I_FRAME_INTERVAL`: a negative value means no key frame after the first one (API 16); today's attempts keep [todayS]. */
    fun keyframeIntervalS(tier: EncoderHints, todayS: Int): Int =
        if (tier == EncoderHints.MODERN) NO_PERIODIC_KEYFRAMES else todayS

    /** `KEY_INTRA_REFRESH_PERIOD` in frames: one second of frames; `null` without the codec feature or below API 24. */
    fun intraRefreshPeriod(fps: Int, sdk: Int, featureSupported: Boolean): Int? =
        if (sdk >= SDK_INTRA_REFRESH && featureSupported) fps.coerceAtLeast(1) else null

    fun encoderLatencyAllowed(sdk: Int): Boolean = sdk >= SDK_ENCODER_LATENCY

    fun decoderLatencyAllowed(sdk: Int): Boolean = sdk >= SDK_DECODER_LATENCY

    fun requestEncoder(format: MediaFormat) {
        format.setInteger(MediaFormat.KEY_LATENCY,1)
        format.setInteger(MediaFormat.KEY_MAX_B_FRAMES,0)
    }
    fun encoderFormat(codec: String, format: MediaFormat): Int? {
        if (!format.containsKey(MediaFormat.KEY_LATENCY)) return null
        val frames=format.getInteger(MediaFormat.KEY_LATENCY)
        if (frames !in 0..1) throw Unavailable(codec,"bestätigte Encoderlatenz $frames Eingaben passt nicht zum Einzelinputvertrag")
        return frames
    }
    /** `KEY_LOW_LATENCY` (decoder, API 30) when the decoder reports `FEATURE_LowLatency`; returns whether it was set. */
    fun requestDecoder(caps: MediaCodecInfo.CodecCapabilities, format: MediaFormat): Boolean {
        val supported=decoderLatencyAllowed(Build.VERSION.SDK_INT) && caps.isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency)
        if (supported) format.setInteger(MediaFormat.KEY_LOW_LATENCY,1)
        return supported
    }
    fun checkDecoderHint(format: MediaFormat, requested: Boolean): Boolean = requested &&
        format.containsKey(MediaFormat.KEY_LOW_LATENCY) && format.getInteger(MediaFormat.KEY_LOW_LATENCY)==1

    data class AccessUnit(val buffer: ByteBuffer, val info: MediaCodec.BufferInfo, val owner: AutoCloseable?=null)
    /** Partial outputs still belong to one input; only a complete AU proves progress. */
    class AccessUnits(private val name: String, private val maxBytes: Int,
        private val allocate: ((Int) -> ScreenCaptureBuffer)?=null) {
        private var partial: ByteBuffer?=null
        private val partialOwner=ScreenCaptureSlot<ScreenCaptureBuffer>()
        private var pts=0L
        private var flags=0
        fun complete(buffer: ByteBuffer, info: MediaCodec.BufferInfo): AccessUnit? {
            if(partial==null && info.flags and MediaCodec.BUFFER_FLAG_PARTIAL_FRAME==0) return AccessUnit(buffer,info)
            val target=partial ?: partialOwner.allocate({allocate?.invoke(maxBytes)}) {owner ->
                val storage=owner?.access {it} ?: ByteBuffer.allocateDirect(maxBytes)
                partial=storage;pts=info.presentationTimeUs;flags=0
                storage
            }
            if(info.presentationTimeUs!=pts || info.size>target.remaining()) throw Unavailable(name,"fragmentierte AU widerspricht FrameStamp oder Größenbudget")
            val source=buffer.duplicate().apply {position(info.offset);limit(info.offset+info.size)}
            target.put(source)
            flags=flags or info.flags
            if(info.flags and MediaCodec.BUFFER_FLAG_PARTIAL_FRAME!=0) {
                if(info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM!=0) throw Unavailable(name,"EOS beendet eine unvollständige AU")
                return null
            }
            val full=MediaCodec.BufferInfo().apply {set(0,target.position(),pts,flags and MediaCodec.BUFFER_FLAG_PARTIAL_FRAME.inv())}
            target.flip()
            val output=partialOwner.transfer {AccessUnit(target,full,it)}
            partial=null
            return output
        }
        fun clear() {partial=null;partialOwner.clear()}
    }

    /** Only one real capture may wait for its exact AU. The raw pixels stay in the owner. */
    class EncoderAdmission(private val name: String) {
        private data class Pending(val pts: Long, val frame: Long, val since: Long)
        private var pending: Pending?=null
        @Synchronized fun expect(pts: Long, frame: Long) {
            check(pending==null && frame>0)
            pending=Pending(pts,frame,System.nanoTime())
        }
        @Synchronized fun output(pts: Long,frame: Long?) {
            val input=pending ?: throw Unavailable(name,"Encoder lieferte eine AU ohne zugehörige Capturereferenz")
            if(input.pts!=pts || input.frame!=frame) throw Unavailable(name,"Encoder-Ausgabe gehört nicht zum ausstehenden FrameStamp")
            pending=null
        }
        @Synchronized fun cancel(pts: Long) {if(pending?.pts==pts) pending=null}
        @Synchronized fun checkProgress() {
            val input=pending ?: return
            if(System.nanoTime()-input.since>=TRANSACTION_NS) throw Unavailable(name,
                "keine passende AU nach einem echten Input innerhalb der Referenzfrist; Einzelinput-Latenz unbestätigt")
        }
    }
}
