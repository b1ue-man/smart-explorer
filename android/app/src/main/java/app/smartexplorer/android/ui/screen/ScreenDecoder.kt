package app.smartexplorer.android.ui.screen

import android.graphics.Bitmap
import app.smartexplorer.android.screen.ScreenFrameLimits
import app.smartexplorer.android.screen.ScreenCodecLatency
import app.smartexplorer.android.screen.ScreenEncoderCodecs
import android.media.MediaCodec
import android.media.MediaCodecInfo.CodecProfileLevel
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Handler
import android.os.Looper
import java.util.concurrent.ConcurrentHashMap
import android.util.Log
import android.view.Surface
import app.smartexplorer.android.api.DecoderOffer
import app.smartexplorer.android.api.ScreenBufferBudget
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
data class DecoderCaps(val hardware: Boolean, val avcProfiles: Set<Int>, val maxWidth: Int, val maxHeight: Int, val pixelAccess: Boolean = false, val maxMacroblocks: Long? = null, val maxMacroblocksPerSecond: Long? = null)

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
    /** The group's `stream` event for its complete native GOP ID, once it arrived. */
    private val streamOf: (Long) -> ScreenViewEvent.Stream?,
    private val surface: () -> Surface?,
    private val listener: Listener,
    private val presentationEpoch: () -> Long = { 0L },
    bufferBudget:()->ScreenBufferBudget? = {ScreenBufferBudget.LEGACY},
    pullBuffers:ScreenPullBuffers? = null,
    /** The viewer's choice „Dekodierer“ and „Codec“: which decoder stage may run a group (BK7b, [ScreenDecoderPolicy]). */
    private val choice: () -> ViewerCodecChoice = { ViewerCodecChoice() },
) {
    interface Listener {
        fun onSize(width: Int, height: Int)
        fun onBitmap(bitmap: Bitmap)
        fun onPicture()
        fun onCompleteFrame(frameId: Long) {}
        fun onBitmapFrame(bitmap: Bitmap, frame: ScreenPresentedFrame) {
            onFrameSource(frame.gopId)
            onBitmap(bitmap)
            onCompleteFrame(frame.frameId)
        }
        /** MediaCodec confirmed this exact output on the still-active Surface. */
        fun onSurfaceFrame(frame: ScreenPresentedFrame) {}
        /** The MediaCodec decoder that runs the picture group: its [codec] key, its name and whether it is a hardware decoder. */
        fun onDecoderStage(codec: String, name: String, hardware: Boolean) {}
        fun onFrameSource(gopId: Long) {}
        /** The decoder failed [consecutive] times in a row without a picture in between. */
        fun onDecodeFailure(consecutive: Int)
        /** A valid stream needs another codec mode; retrying its keyframe cannot admit it. */
        fun onHybridUnavailable(gopId: Long, reason: String) {onDecodeFailure(1)}
        fun onEnded()
    }

    @Volatile private var running = false
    private var thread: Thread? = null
    @Volatile private var codec: MediaCodec? = null
    private var codecGop = -1L
    private var latestGop = 0L
    private var codecWidth=0
    private var codecHeight=0
    /** The Surface the running decoder was configured with. */
    private var codecSurface: Surface? = null
    /** The codec key of the running decoder (`h264`, `hevc`, `av1`); only H.264 is checked with `ScreenFrameLimits.checkAvc`. */
    private var activeCodec=ScreenEncoderCodecs.H264
    private val avcChecked get()=activeCodec==ScreenEncoderCodecs.H264
    private var heldIndex = -1
    private val pulls=pullBuffers ?: ScreenPullBuffers(bufferBudget)
    private val staging get()=pulls.buffer
    private var bitmap: Bitmap? = null
    private var failures = 0
    private var awaitingKeyframe = true
    private var hybrid: ScreenHybridDecoder? = null
    private var hybridGop = -1L
    private var hybridFrameEpoch=0L
    private var rejectedHybridGop:Long?=null
    private var payloadOffset=HEADER_BYTES
    private var frameId: Long?=null
    private var frameEpoch=0L
    private var keyQueued=false
    private var referenceWaitNs=0L
    private data class Submission(val frame: ScreenPresentedFrame, val keyframe: Boolean)
    private val submitted=mutableMapOf<Long,Submission>()
    private val rendering=ConcurrentHashMap<Long,ScreenPresentedFrame>()

    fun start() {
        synchronized(decoders) {
        if(thread?.isAlive==true) return
        val previous=decoders.put(sessionId,this)?.takeIf {it!==this}
        previous?.stop()
        running = true
        thread = Thread({
            previous?.thread?.join()
            pulls.takeFrom(previous?.pulls)
            run()
        }, "screen-decode-$sessionId").also { it.start() }
        }
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
            hybrid?.close();hybrid=null;bitmap=null
            decoders.remove(sessionId,this)
            listener.onEnded()
        }
    }

    private fun loop() {
        while (running) {
            if(!pulls.prepare()) {
                val status=source.pull(staging,PULL_WAIT_MS)
                if(status==PULL_ENDED || status<PULL_TOO_SMALL) return
                if(status==PULL_TOO_SMALL) Thread.sleep(STREAM_POLL_MS)
                continue
            }
            val active = codec
            var dst = staging
            if (active != null) {
                drain(active)
                if (codec !== active) continue
                if(awaitingKeyframe && keyQueued && System.nanoTime()-referenceWaitNs>2_000_000_000L) { fail(); continue }
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
        val target=pulls.forPayload(payloadLen) ?: run {fail();return}
        val n = source.pull(target, PULL_WAIT_MS)
        if (n > HEADER_BYTES) accept(staging, n)
    }

    private fun accept(src: ByteBuffer, n: Int) {
        val pts = src.getLong(0)
        val flags = src.getInt(8)
        var len = src.getInt(12)
        if (len <= 0 || len != n - HEADER_BYTES || n > src.capacity()) { fail(); return }
        payloadOffset=HEADER_BYTES
        frameEpoch=presentationEpoch()
        frameId=null
        // Low-16 flag bits cannot identify a pulled record after a GOP wrap.
        if(flags and FLAG_GOP_ID == 0) return
        if(len<8) { fail(); return }
        val gop=src.getLong(payloadOffset)
        // Flip the sign bit to compare the native u64 IDs without narrowing them.
        if((gop xor Long.MIN_VALUE) < (latestGop xor Long.MIN_VALUE)) return
        payloadOffset+=8; len-=8
        if(flags and FLAG_FRAME_ID != 0) {
            if(len<8) { fail(); return }
            frameId=src.getLong(payloadOffset).takeIf { it>0 }
            payloadOffset+=8; len-=8
        }
        if(!pulls.acceptsPayload(len,flags and FLAG_TILES!=0)) {fail();return}
        if (flags and FLAG_HYBRID != 0) {
            if(flags and FLAG_NEW_STREAM == 0 && hybridGop!=gop) return
            try {
                val payload=ByteArray(len); src.position(payloadOffset); src.get(payload)
                if (flags and FLAG_NEW_STREAM != 0) {
                    require(flags and FLAG_KEYFRAME != 0)
                    val info = awaitStream(gop) ?: return
                    if(rejectedHybridGop==info.gop) return
                    frameEpoch=presentationEpoch()
                    releaseCodec(); hybrid?.close();hybrid=null;bitmap=null
                    hybrid = ScreenHybridDecoder(info.width,info.height,pulls.budget,choice().decoder) {running}; hybridGop=gop
                }
                if(rejectedHybridGop==hybridGop) return
                require(hybridGop==gop)
                if(payload.size>=25 && payload[24]==0.toByte()) hybridFrameEpoch=presentationEpoch()
                hybrid?.accept(payload,flags and FLAG_KEYFRAME != 0,flags and FLAG_MORE_PARTS != 0)?.let {
                    val completeId = hybrid?.completeFrameId ?: error("Missing complete hybrid identity")
                    require(frameId == null || frameId == completeId)
                    awaitingKeyframe=false; failures=0; listener.onSize(it.width,it.height)
                    listener.onBitmapFrame(it, ScreenPresentedFrame(sessionId, hybridGop,
                        frameId ?: completeId, hybridFrameEpoch))
                    listener.onPicture()
                }
            } catch(e: ScreenCodecLatency.Unavailable) {
                if(running) {
                    val rejected=hybridGop
                    rejectedHybridGop=rejected
                    awaitingKeyframe=true;hybrid?.close();hybrid=null;hybridGop=-1L
                    releaseCodec()
                    listener.onHybridUnavailable(rejected,e.message.orEmpty())
                }
            } catch (_: RuntimeException) {if(running) fail()}
            return
        }
        if (flags and FLAG_TILES != 0) {
            drawBitmap(src, gop, len)
            return
        }
        val keyframe = flags and FLAG_KEYFRAME != 0
        val newStream = flags and FLAG_NEW_STREAM != 0
        if(newStream && !keyframe) { fail(); return }
        if (awaitingKeyframe && !keyQueued && !keyframe) return
        if (codec != null && codecGop != gop && !newStream) return
        if (codec != null && !newStream) {
            queue(src, pts, keyframe, len)
            return
        }
        if (!newStream && !keyframe) return
        val payload = ByteArray(len)
        src.position(payloadOffset)
        src.get(payload)
        val info = awaitStream(gop) ?: return
        frameEpoch=presentationEpoch()
        // The same group restarts at its key frame (the core's queue stalled or a record broke,
        // `viewer_sink.rs`): same encoder, codec, size and Surface, so the running decoder is
        // flushed and keeps going instead of being torn down and opened again.
        if (restartInPlace(info, gop)) {
            queueBytes(payload, pts, keyframe)
            return
        }
        // A new group (or a decoder that failed): the old decoder goes, its buffers with it.
        releaseCodec()
        hybrid?.close();hybrid=null;hybridGop=-1L
        if (!configure(info, gop, payload)) return
        queueBytes(payload, pts, keyframe)
    }

    private fun queue(src: ByteBuffer, pts: Long, keyframe: Boolean, len: Int) {
        val c = codec ?: return
        val key = if (keyframe) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0
        try {
            if(avcChecked) ScreenFrameLimits.checkAvc(src,payloadOffset,len,codecWidth,codecHeight,pulls.budget)
            else pulls.budget.checkedBytes(codecWidth,codecHeight)
            if (src === staging) {
                val payload = ByteArray(len)
                src.position(payloadOffset)
                src.get(payload)
                queueBytes(payload, pts, keyframe)
            } else {
                c.queueInputBuffer(heldIndex, payloadOffset, len, pts, key)
                rememberSubmission(pts,keyframe)
                if(keyframe && awaitingKeyframe) { keyQueued=true; referenceWaitNs=System.nanoTime() }
                heldIndex = -1
            }
        } catch (e: IllegalArgumentException) {
            fail()
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
            if(avcChecked) ScreenFrameLimits.checkAvc(payload,codecWidth,codecHeight,pulls.budget)
            else pulls.budget.checkedBytes(codecWidth,codecHeight)
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
            rememberSubmission(pts,keyframe)
            if(keyframe && awaitingKeyframe) { keyQueued=true; referenceWaitNs=System.nanoTime() }
        } catch (e: IllegalArgumentException) {
            fail()
        } catch (e: MediaCodec.CodecException) {
            fail()
        } catch (e: IllegalStateException) {
            fail()
        }
    }

    /**
     * Restarts the running decoder at the key frame of its own group [gop] with `flush()` (synchronous mode: every
     * buffer index is revoked, the codec stays configured and Executing). MediaCodec allows this only for data of
     * the same format and only once the decoder gave output (before that the parameter sets would have to be
     * submitted again), so anything else (another group, codec, size or Surface, no output yet, a refused flush)
     * returns false and the caller opens a new decoder as before.
     */
    private fun restartInPlace(info: ScreenViewEvent.Stream, gop: Long): Boolean {
        val c = codec ?: return false
        if (gop != codecGop || awaitingKeyframe || info.codec != activeCodec ||
            info.width != codecWidth || info.height != codecHeight) return false
        val target = codecSurface ?: return false
        if (surface() !== target) return false
        try {
            c.flush()
        } catch (_: IllegalStateException) {
            return false
        }
        heldIndex = -1
        submitted.clear(); rendering.clear(); keyQueued=false
        awaitingKeyframe = true
        return true
    }

    private fun awaitStream(gop: Long): ScreenViewEvent.Stream? {
        var waited = 0L
        while (running && waited <= STREAM_WAIT_MS) {
            streamOf(gop)?.takeIf { it.gop==gop }?.let { latestGop=gop; return it }
            Thread.sleep(STREAM_EVENT_POLL_MS)
            waited += STREAM_EVENT_POLL_MS
        }
        if(running) fail()
        return null
    }

    private fun rememberSubmission(pts:Long,keyframe:Boolean) {
        // Proof metadata cannot grow when a codec consumes inputs without output.
        // An evicted timestamp remains unrendered and cannot release an input fence.
        if(submitted.size>=32 && !submitted.containsKey(pts)) submitted.keys.firstOrNull()?.let(submitted::remove)
        submitted[pts]=Submission(ScreenPresentedFrame(sessionId,codecGop,frameId ?: 0L,frameEpoch),keyframe)
    }

    private fun configure(info: ScreenViewEvent.Stream, gop: Long, payload: ByteArray): Boolean {
        val target=surface() ?: run { fail(); return false }
        val checked=try { pulls.budget.checkedBytes(info.width,info.height) } catch (_: IllegalArgumentException) { fail(); return false }
        // H.264, HEVC or AV1 (B-39): the group's codec names the MIME type. Only 4:2:0 is offered, so another profile cannot arrive.
        val codecKey=info.codec
        val mime=ScreenEncoderCodecs.mime(codecKey) ?: run { fail(); return false }
        if(codecKey!=ScreenEncoderCodecs.H264 && info.profile!=null) { fail(); return false }
        // The in-band parameter sets of HEVC and AV1 were checked by the core before the picture reached this thread (`viewer_sink.rs`).
        if(codecKey==ScreenEncoderCodecs.H264) try {ScreenFrameLimits.checkAvc(payload,info.width,info.height,pulls.budget)} catch (_: RuntimeException) {fail(); return false}
        codecWidth=info.width;codecHeight=info.height
        var avcProfile: Int?=null
        for(at in 0 until (if(codecKey==ScreenEncoderCodecs.H264) (payload.size-4).coerceAtLeast(0) else 0)) {
            if(payload[at]==0.toByte() && payload[at+1]==0.toByte() && payload[at+2]==1.toByte() && payload[at+3].toInt() and 31==7) {
                avcProfile=payload[at+4].toInt() and 255; break
            }
        }
        val everyDecoder=MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.filter {
            !it.isEncoder && !it.isAlias && it.supportedTypes.any { type -> type.equals(mime,true) }
        }
        // „Dekodierer“ and the hardware rule of HEVC and AV1 (`ScreenDecoderPolicy`); the hardware decoder runs first.
        val hardwareExists=everyDecoder.any { it.isHardwareAccelerated }
        val wanted=choice()
        val choices=everyDecoder.filter { ScreenDecoderPolicy.allows(wanted,codecKey,it.isHardwareAccelerated,hardwareExists) }
            .sortedByDescending { it.isHardwareAccelerated }
        val mainProfile=when(codecKey) {
            ScreenEncoderCodecs.HEVC -> CodecProfileLevel.HEVCProfileMain
            ScreenEncoderCodecs.AV1 -> CodecProfileLevel.AV1ProfileMain8
            else -> null
        }
        for(candidate in choices) {
            val caps=candidate.getCapabilitiesForType(mime)
            if(caps.videoCapabilities?.isSizeSupported(info.width,info.height)!=true) continue
            val profiles=caps.profileLevels.map { it.profile }
            if(avcProfile==100 && CodecProfileLevel.AVCProfileHigh !in profiles) continue
            if(mainProfile!=null && mainProfile !in profiles) continue
            for(hints in listOf(true,false)) {
                val format=MediaFormat.createVideoFormat(mime,info.width,info.height).apply {
                    setInteger(MediaFormat.KEY_MAX_INPUT_SIZE,minOf((checked.toLong()*3/4).coerceAtLeast(64*1024L),pulls.budget.recordBytes.toLong()).toInt())
                    // KEY_LOW_LATENCY (API 30) where the decoder reports FEATURE_LowLatency; a refusal falls back to the attempt without hints.
                    if(hints) ScreenCodecLatency.requestDecoder(caps, this)
                }
                var created: MediaCodec?=null
                try {
                    if(!caps.isFormatSupported(format)) continue
                    created=MediaCodec.createByCodecName(candidate.name)
                    created.configure(format,target,null,0)
                    created.setOnFrameRenderedListener({ renderedCodec, pts, _ ->
                        if (running && codec === renderedCodec && surface() === target) {
                            rendering.remove(pts)?.let(listener::onSurfaceFrame)
                        }
                    }, Handler(Looper.getMainLooper()))
                    created.start()
                    codec=created; codecGop=gop; awaitingKeyframe=true; activeCodec=codecKey; codecSurface=target
                    listener.onDecoderStage(codecKey,candidate.name,candidate.isHardwareAccelerated)
                    return true
                } catch(e: RuntimeException) {
                    created?.release(); Log.w(TAG,"Decoder ${candidate.name} abgelehnt: ${e.javaClass.simpleName}")
                }
            }
        }
        fail(); return false
    }

    private fun drain(c: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        try {
            while (true) {
                val index = c.dequeueOutputBuffer(info, if(awaitingKeyframe) INPUT_WAIT_US else 0)
                when {
                    index >= 0 -> {
                        val completed=submitted.remove(info.presentationTimeUs)
                        if(awaitingKeyframe && completed?.keyframe != true) { c.releaseOutputBuffer(index,false); fail(); return }
                        listener.onFrameSource(codecGop)
                        if (running && completed != null && info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0) {
                            // Exact PTS binding; absent/delayed platform callbacks leave input blocked.
                            // This is only a proof cache, never the decoder queue. Evicted output
                            // remains unacknowledged if its informational callback arrives late.
                            if (rendering.size >= ScreenSessionState.STREAM_CACHE) {
                                rendering.keys.minOrNull()?.let(rendering::remove)
                            }
                            rendering[info.presentationTimeUs]=completed.frame
                            c.releaseOutputBuffer(index, true)
                        } else c.releaseOutputBuffer(index, false)
                        failures = 0
                        awaitingKeyframe = false
                        listener.onPicture()

                    }
                    index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                        ScreenFrameLimits.checkOutput(c.outputFormat,codecWidth,codecHeight,pulls.budget)
                        listener.onSize(codecWidth,codecHeight)
                    }
                    else -> return
                }
            }
        } catch (e: IllegalArgumentException) {
            fail()
        } catch (e: MediaCodec.CodecException) {
            fail()
        } catch (e: IllegalStateException) {
            fail()
        }
    }

    private fun drawBitmap(src: ByteBuffer, gop: Long, len: Int) {
        val info = awaitStream(gop) ?: return
        frameEpoch=presentationEpoch()
        val expected=try { pulls.budget.checkedBytes(info.width,info.height) } catch (_: IllegalArgumentException) { fail(); return }
        if (len != expected) { fail(); return }
        val pixels=src.duplicate().apply {position(payloadOffset);limit(payloadOffset+len)}
        releaseCodec(); hybrid?.close(); hybrid=null; hybridGop=-1L
        var target = bitmap
        if (target == null || target.width != info.width || target.height != info.height) {
            target = Bitmap.createBitmap(info.width, info.height, Bitmap.Config.ARGB_8888)
            bitmap = target
        }
        target.copyPixelsFromBuffer(pixels)
        listener.onBitmapFrame(target,ScreenPresentedFrame(sessionId,info.gop,frameId ?: 0L,frameEpoch))
        awaitingKeyframe=false; failures=0

    }

    private fun fail() {
        awaitingKeyframe=true
        hybrid?.close(); hybrid=null; hybridGop=-1L
        failures++
        releaseCodec()
        listener.onDecodeFailure(failures)
    }

    private fun releaseCodec() {
        heldIndex = -1
        codecGop = -1L
        submitted.clear(); rendering.clear(); keyQueued=false
        codecSurface = null
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
        private val decoders=ConcurrentHashMap<Long,ScreenDecoder>()
        private const val TAG = "ScreenDecoder"
        private const val MIME = "video/avc"
        const val HEADER_BYTES = 16
        const val FLAG_KEYFRAME = 1
        const val FLAG_TILES = 2
        const val FLAG_NEW_STREAM = 4
        const val FLAG_HYBRID = 0x10
        const val FLAG_MORE_PARTS = 0x20
        const val FLAG_FRAME_ID = 0x80
        const val FLAG_GOP_ID = 0x100
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
        /** How often [awaitStream] looks for the group's `stream` event: it holds the group's first picture meanwhile. */
        private const val STREAM_EVENT_POLL_MS = 2L

        /** The decoders this phone offers the host: H.264 per profile and hardware, and always `tiles`. */
        fun offers(): List<DecoderOffer> {
            val caps = try {
                MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
                    .filter { !it.isEncoder && MIME in it.supportedTypes }
                    .flatMap { info ->
                        val c = info.getCapabilitiesForType(MIME)
                        val profiles=setOf(CodecProfileLevel.AVCProfileHigh,CodecProfileLevel.AVCProfileConstrainedBaseline,CodecProfileLevel.AVCProfileBaseline)
                        ScreenFrameLimits.offers(c,profiles).map { limit -> DecoderCaps(
                            hardware=info.isHardwareAccelerated,avcProfiles=setOf(limit.profile),
                            maxWidth=limit.width,maxHeight=limit.height,
                            pixelAccess=android.media.MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible in c.colorFormats,
                            maxMacroblocks=limit.avc?.blocks,maxMacroblocksPerSecond=limit.avc?.blocksPerSecond,
                        ) }
                    }
            } catch (e: RuntimeException) {
                emptyList()
            }
            return offersFor(caps)
        }

        /**
         * The decoders for the viewer's own choice (BK7b, B-39, 4.10): H.264 in the stage „Dekodierer“ allows, then HEVC
         * and AV1 (4:2:0, `main`) as [ScreenDecoderPolicy] lets them run, `tiles` last. The offers of
         * a preview stay [offers].
         */
        fun offers(choice: ViewerCodecChoice): List<DecoderOffer> =
            ScreenDecoderPolicy.combine(offers(), ScreenDecoderPolicy.videoOffers(videoDecoderCaps(), choice), choice)

        /** The HEVC and AV1 decoders of MediaCodec that name the `main` profile and report their size limits. */
        fun videoDecoderCaps(): List<VideoDecoderCaps> = try {
            MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.filter { !it.isEncoder && !it.isAlias }.flatMap { info ->
                ScreenEncoderCodecs.VIDEO.mapNotNull { codec ->
                    val mime = ScreenEncoderCodecs.mime(codec) ?: return@mapNotNull null
                    if (info.supportedTypes.none { it.equals(mime, true) }) return@mapNotNull null
                    val caps = info.getCapabilitiesForType(mime)
                    val video = caps.videoCapabilities ?: return@mapNotNull null
                    val main = ScreenEncoderCodecs.profile(codec, "main") ?: return@mapNotNull null
                    if (caps.profileLevels.none { it.profile == main }) return@mapNotNull null
                    VideoDecoderCaps(
                        codec, info.isHardwareAccelerated,
                        minOf(video.supportedWidths.upper, ScreenFrameLimits.MAX_SIDE),
                        minOf(video.supportedHeights.upper, ScreenFrameLimits.MAX_SIDE),
                    )
                }
            }
        } catch (e: RuntimeException) {
            emptyList()
        }

        /** [offers] for the given decoders. */
        fun offersFor(codecs: List<DecoderCaps>): List<DecoderOffer> {
            val wanted = listOf(
                "high" to setOf(CodecProfileLevel.AVCProfileHigh),
                "cb" to setOf(CodecProfileLevel.AVCProfileConstrainedBaseline, CodecProfileLevel.AVCProfileBaseline),
            )
            val h264 = wanted.flatMap { (profile, accepted) ->
                listOf(true, false).flatMap { hardware ->
                    codecs.filter { it.hardware == hardware && it.avcProfiles.any { p -> p in accepted } }.map { caps ->
                        DecoderOffer(
                            codec = "h264",
                            profile = profile,
                            hardware = hardware,
                            pixelAccess = caps.pixelAccess,
                            maxWidth = caps.maxWidth.takeIf { it > 0 },
                            maxHeight = caps.maxHeight.takeIf { it > 0 },
                            maxMacroblocks=caps.maxMacroblocks,maxMacroblocksPerSecond=caps.maxMacroblocksPerSecond,
                        )
                    }
                }
            }
            return h264.distinct() + DecoderOffer(codec = "tiles",pixelAccess=true)
        }
    }
}
