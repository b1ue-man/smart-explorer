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
    fun requestDecoder(caps: MediaCodecInfo.CodecCapabilities, format: MediaFormat): Boolean {
        val supported=Build.VERSION.SDK_INT>=30 && caps.isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency)
        if (supported) format.setInteger(MediaFormat.KEY_LOW_LATENCY,1)
        return supported
    }
    fun checkDecoderHint(format: MediaFormat, requested: Boolean): Boolean = requested &&
        format.containsKey(MediaFormat.KEY_LOW_LATENCY) && format.getInteger(MediaFormat.KEY_LOW_LATENCY)==1

    data class AccessUnit(val buffer: ByteBuffer, val info: MediaCodec.BufferInfo)
    /** Partial outputs still belong to one input; only a complete AU proves progress. */
    class AccessUnits(private val name: String, private val maxBytes: Int) {
        private var partial: ByteBuffer?=null
        private var pts=0L
        private var flags=0
        fun complete(buffer: ByteBuffer, info: MediaCodec.BufferInfo): AccessUnit? {
            if(partial==null && info.flags and MediaCodec.BUFFER_FLAG_PARTIAL_FRAME==0) return AccessUnit(buffer,info)
            val target=partial ?: ByteBuffer.allocateDirect(maxBytes).also {partial=it;pts=info.presentationTimeUs;flags=0}
            if(info.presentationTimeUs!=pts || info.size>target.remaining()) throw Unavailable(name,"fragmentierte AU widerspricht FrameStamp oder Größenbudget")
            val source=buffer.duplicate().apply {position(info.offset);limit(info.offset+info.size)}
            target.put(source)
            flags=flags or info.flags
            if(info.flags and MediaCodec.BUFFER_FLAG_PARTIAL_FRAME!=0) {
                if(info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM!=0) throw Unavailable(name,"EOS beendet eine unvollständige AU")
                return null
            }
            val full=MediaCodec.BufferInfo().apply {set(0,target.position(),pts,flags and MediaCodec.BUFFER_FLAG_PARTIAL_FRAME.inv())}
            partial=null;target.flip()
            return AccessUnit(target,full)
        }
        fun clear() {partial=null}
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
