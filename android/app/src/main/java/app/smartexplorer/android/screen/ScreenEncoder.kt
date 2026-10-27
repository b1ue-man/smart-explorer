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
    val hybrid: Boolean = false,
    val codec: String = "h264",
)

/**
 * H.264 of this phone as screen host (C2 Android): the MediaCodec encoder with an input surface
 * that the virtual display of MediaProjection or the test picture draws into. Hardware encoders
 * first (`isHardwareAccelerated`), then the system's software encoder. Every key frame carries
 * SPS and PPS in front, so each picture group starts decodable; key frames come when the core asks
 * (`requestKeyFrame`) and at most every [KEYFRAME_INTERVAL_S]. Nothing of a picture is logged.
 */
internal class ScreenEncoder(private val config: EncoderConfig, private val budget: ScreenCaptureBudget, private val sink: PictureSink) {
    // All allocating bookkeeping precedes the first platform/native resource.
    private val construction=ScreenCaptureConstruction<MediaCodec,Surface>({it.release()},{it.release()})
    private val surfaceOwners=ScreenSurfaceOwners()
    private val surfaceLease: ScreenCaptureLease?
    private val codec: MediaCodec
    private val inputSurface: Surface?
    val surface: Surface get() = inputSurface ?: error("RGBA encoder has no input surface")
    val name: String
    val hardware: Boolean

    @Volatile
    private var running = true
    @Volatile private var failureHandler: ((String) -> Unit)? = null
    @Volatile private var lastFailure: String? = null
    fun setFailureHandler(handler: (String) -> Unit) { failureHandler=handler; lastFailure?.let(handler) }
    private val pending = ScreenCaptureInputs<ScreenCaptureBuffer>()
    @Volatile private var forceKeyframe = true
    private var lastPixels: ScreenCaptureBuffer? = null
    private var lastCapturePts=0L
    private fun nextCapturePts(): Long = maxOf(System.nanoTime()/1000,Math.addExact(lastCapturePts,1)).also {lastCapturePts=it}
    private var waitingKeyframe = true
    private lateinit var bitrateRange: android.util.Range<Int>
    private var codecConfig: ScreenCaptureBuffer? = null
    private var joined: ScreenCaptureBuffer? = null
    private val drain: Thread
    private val admission:ScreenCodecLatency.EncoderAdmission
    private val accessUnits:ScreenCodecLatency.AccessUnits

    init {
        try {
            surfaceLease=construction.ownClaim(if(!config.hybrid) budget.claim(ScreenCaptureBudget.NATIVE_SURFACE,
                ScreenFrameLimits.checkedBytes(config.width,config.height).toLong()) else null)
            val opened = open(config,construction)
            codec = opened.first
            bitrateRange = opened.third
            val info = opened.second
            name = info.name
            admission=ScreenCodecLatency.EncoderAdmission(name)
            accessUnits=ScreenCodecLatency.AccessUnits(name,MAX_ACCESS_UNIT) {size -> budget.buffer(ScreenCaptureBudget.CODEC_OUTPUT,size)}
            hardware = info.isHardwareAccelerated
            inputSurface = construction.ownSurface(if (config.hybrid) null else codec.createInputSurface())
            codec.start()
            drain = thread(start = false, name = "screen-encoder", isDaemon = true) {
                if(construction.enterRuntime()) {
                    try { drainLoop() } catch(e: RuntimeException) {
                        running=false
                        val reason=if(e is ScreenCodecLatency.Unavailable) e.message.orEmpty() else "MediaCodec-Ausgabe ($name): ${e.message ?: e.javaClass.simpleName}"
                        lastFailure=reason; failureHandler?.invoke(reason); Log.w(TAG,reason)
                    } finally {finishResources()}
                }
            }
            drain.start()
        } catch(failure: Throwable) {
            running=false
            construction.abort(failure)
        }
    }
    /** Register before starting a Canvas producer, including spontaneous drain failure. */
    fun retainSurface(): ScreenCaptureLease = surfaceOwners.retain(checkNotNull(surfaceLease)) {running}

    fun requestKeyFrame() { forceKeyframe=true; parameters { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) } }

    fun setBitrate(bps: Int) = parameters { putInt(MediaCodec.PARAMETER_KEY_VIDEO_BITRATE, bps.coerceIn(bitrateRange.lower, bitrateRange.upper)) }

    private fun parameters(fill: Bundle.() -> Unit) {
        try {
            codec.setParameters(Bundle().apply(fill))
        } catch (e: RuntimeException) {
            Log.w(TAG, "Kodiererparameter nicht gesetzt: ${e.message}")
        }
    }

    fun release() {
        surfaceOwners.beginRelease()
        running = false
        try {
            codec.signalEndOfInputStream()
        } catch (e: IllegalStateException) {
            // Already stopped.
        }
        if(Thread.currentThread()!==drain) drain.join(DRAIN_JOIN_MS)
        try {
            codec.stop()
        } catch (e: IllegalStateException) {
            // Already stopped.
        }
        if(Thread.currentThread()!==drain) drain.join(DRAIN_JOIN_MS)
        check(!drain.isAlive) {"MediaCodec-Ausgabethread hat nicht angehalten; Bildbesitz bleibt bis Threadende bestehen"}
    }

    @Synchronized private fun finishResources() {
        // A Codec error can finish this drain before the source is stopped.
        // Keep every platform resource and its claim until its real worker ends.
        surfaceOwners.awaitEnded()
        try {codec.release()} finally {
            inputSurface?.release()
            pending.clear()
            lastPixels?.close();lastPixels=null
            codecConfig?.close();codecConfig=null;joined?.close();joined=null
            accessUnits.clear();surfaceLease?.close()
        }
    }

    private fun drainLoop() {
        val info = MediaCodec.BufferInfo()
        while (running) {
            if(config.hybrid) admission.checkProgress()
            val index = try {
                codec.dequeueOutputBuffer(info, DEQUEUE_TIMEOUT_US)
            } catch (e: IllegalStateException) {
                if(running) throw e else return
            }
            if (index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                val output=codec.outputFormat
                if(config.hybrid) ScreenCodecLatency.encoderFormat(name,output)
                val sps=output.getByteBuffer("csd-0")
                val pps=output.getByteBuffer("csd-1")
                if (sps != null) {
                    val a=sps.duplicate(); val b=pps?.duplicate()
                    val configBytes=Math.addExact(a.remaining(),b?.remaining() ?: 0)
                    require(configBytes<=MAX_ACCESS_UNIT) {"MediaCodec-Parameter überschreiten das AU-Budget"}
                    replaceHeaders(configBytes) {target -> target.put(a);if(b!=null) target.put(b)}
                }
            }
            if (index < 0) continue
            try {
                val buffer = codec.getOutputBuffer(index) ?: continue
                deliver(buffer, info)
            } finally {
                codec.releaseOutputBuffer(index, false)
            }
            if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) {
                if(config.hybrid && running) throw ScreenCodecLatency.Unavailable(name,"unerwartetes Encoder-EOS beendet die Capturereferenz")
                return
            }
        }
    }

    private fun deliver(buffer: ByteBuffer, info: MediaCodec.BufferInfo) {
        if (info.size <= 0) return
        require(info.offset >= 0 && info.size <= MAX_ACCESS_UNIT && info.offset.toLong() + info.size <= buffer.capacity()) { "Ungültiger MediaCodec-Ausgabebereich" }
        if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
            replaceHeaders(info.size) {target -> target.put(buffer.duplicate().apply {position(info.offset);limit(info.offset+info.size)})}
            return
        }
        val complete=accessUnits.complete(buffer,info) ?: return
        try {deliverComplete(complete.buffer,complete.info)} finally {complete.owner?.close()}
    }
    private fun deliverComplete(buffer: ByteBuffer, info: MediaCodec.BufferInfo) {
        val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
        if(key) {
            val unit=buffer.duplicate().apply {position(info.offset);limit(info.offset+info.size)}.slice()
            require(nal(unit,5)!=null) { "Schlüsselbild enthält kein IDR" }
            if(codecConfig==null) validateHeaders(unit)
        }
        if(config.hybrid) admission.output(info.presentationTimeUs,pending.peek(info.presentationTimeUs)?.frameId)
        if (waitingKeyframe && !key) {pending.take(info.presentationTimeUs)?.close();throw IllegalStateException("MediaCodec lieferte kein IDR für die erste Capturereferenz")}
        if (key) { waitingKeyframe = false; forceKeyframe=false }
        val config = codecConfig
        if (!key || config == null) {
            push(buffer, info.offset, info.size, info.presentationTimeUs, key)
            return
        }
        val total = Math.addExact(config.capacity, info.size)
        require(total <= MAX_ACCESS_UNIT) { "H.264-Referenz größer als 16 MiB" }
        val output=joined?.takeIf {it.capacity>=total} ?: budget.buffer(ScreenCaptureBudget.CODEC_OUTPUT,total).also {
            joined?.close();joined=it
        }
        output.access {target ->
            config.access {target.put(it)}
            target.put(buffer.duplicate().apply {limit(info.offset+info.size);position(info.offset)})
            push(target,0,total,info.presentationTimeUs,true)
        }
    }

    private fun replaceHeaders(length: Int, fill:(ByteBuffer)->Unit) {
        val next=budget.buffer(ScreenCaptureBudget.CODEC_OUTPUT,length)
        try {next.access {target -> fill(target);target.flip();validateHeaders(target)}}
        catch(error: Throwable) {next.close();throw error}
        val previous=codecConfig;codecConfig=next;previous?.close()
    }

    private fun nal(bytes: ByteBuffer, kind: Int): Int? {
        var at=0
        while(at+3<bytes.limit()) {
            if(bytes.get(at)==0.toByte() && bytes.get(at+1)==0.toByte() && bytes.get(at+2)==1.toByte()
                && bytes.get(at+3).toInt() and 31==kind) return at+3
            at++
        }
        return null
    }

    private fun validateHeaders(bytes: ByteBuffer) {
        // This is the local capture encoder; accepted viewer limits are checked by the native producer.
        budget.claim(ScreenCaptureBudget.METADATA,131_080).use {
            ScreenFrameLimits.checkAvc(bytes,0,bytes.limit(),config.width,config.height,null)
        }
        val sps=nal(bytes,7) ?: error("H.264 ohne Annex-B-SPS")
        require(sps+3<bytes.limit())
        val profile=bytes.get(sps+1).toInt() and 255; val constraints=bytes.get(sps+2).toInt() and 255
        val cb=profile==66 && constraints and 0x40 != 0
        require(if(config.profile=="cb") cb else profile==100 || cb) { "MediaCodec lieferte ein nicht ausgehandeltes H.264-Profil" }
        Log.i(TAG,"Tatsächliches H.264-Profil=$profile Einschränkungen=$constraints Level=${bytes.get(sps+3).toInt() and 255}")
    }

    /** One request, one encoder input and the exact RGBA reference of that input. */
    @Synchronized fun queueRgba(rgba: ScreenCaptureBuffer, frameId: Long): Boolean {
        if (!config.hybrid || !running || pending.isPending()) return false
        require(rgba.capacity.toLong()==config.width.toLong()*config.height*4 && rgba.budget===budget)
        if(!forceKeyframe && lastPixels?.samePixels(rgba)==true) {
            val pts=nextCapturePts()
            budget.buffer(ScreenCaptureBudget.TRANSPORT,20).use {packet -> packet.access {target ->
                target.putInt(config.width);target.putInt(config.height);target.putLong(frameId);target.putInt(0)
                check(sink.push(target,0,20,pts,false)>=0)
            }}
            return true
        }
        var index=-1
        repeat(100) { if(index<0 && running) index=codec.dequeueInputBuffer(10_000) }
        if(index<0) return false
        val image=codec.getInputImage(index) ?: error("Kodierer bietet kein beschreibbares YUV-Bild")
        // MediaCodec's Image.getPlanes() copies its array on every call: read it once.
        val planes=image.planes
        require(planes.size==3 && image.cropRect.width()==config.width && image.cropRect.height()==config.height)
        val crop=image.cropRect
        fun plane(at: Int)=ScreenPlane(planes[at].buffer,planes[at].rowStride,planes[at].pixelStride)
        rgba.access {pixels -> ScreenRgbaPlanes.rgbaToYuv420(pixels,config.width,config.height,plane(0),plane(1),plane(2),crop.left,crop.top)}
        val pts=nextCapturePts()
        val capture=ScreenCaptureInput(pts,frameId) {rgba.retain()}
        var nextPixels:ScreenCaptureBuffer?=null
        var published=false
        try {
            pending.expect(capture)
            published=true
            admission.expect(pts,frameId)
            nextPixels=rgba.retain()
            codec.queueInputBuffer(index,0,config.width*config.height*3/2,pts,0)
        } catch(failure: Throwable) {
            val rollbackInput=!published || pending.cancel(capture)
            admission.cancel(pts)
            // A taken input belongs to push() until its own finally, even if
            // MediaCodec reports a queue failure after starting that output.
            try {nextPixels?.close()} finally {if(rollbackInput) capture.close()}
            throw failure
        }
        val previous=lastPixels;lastPixels=nextPixels;previous?.close()
        return true
    }

    private fun push(data: ByteBuffer, offset: Int, length: Int, pts: Long, key: Boolean) {
        if (!this.config.hybrid) { sink.push(data,offset,length,pts,key); return }
        val capture=pending.take(pts) ?: error("Videoframe ohne passende Capturereferenz")
        try {
            val total=Math.addExact(Math.addExact(20,capture.pixels.capacity),length)
            budget.buffer(ScreenCaptureBudget.TRANSPORT,total).use {packet -> packet.access {target ->
                target.putInt(this.config.width);target.putInt(this.config.height);target.putLong(capture.frameId);target.putInt(length)
                capture.pixels.access {target.put(it)}
                val video=data.duplicate();video.position(offset);video.limit(Math.addExact(offset,length));target.put(video)
                check(sink.push(target,0,total,pts,key)>=0) {"Capturereferenz vom Core abgelehnt"}
            }}
        } finally {capture.close()}
    }

    companion object {
        private const val TAG = "SmartExplorerScreen"
        const val KEYFRAME_INTERVAL_S = 10
        private const val MAX_ACCESS_UNIT = 16 * 1024 * 1024
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

        private fun pick(profile: String): MediaCodecInfo? = encoders().firstOrNull { supports(it, profile) }

        /** Each configuration is checked against the actual profile/level and size/rate pair. */
        private fun open(config: EncoderConfig, construction: ScreenCaptureConstruction<MediaCodec,Surface>): Triple<MediaCodec, MediaCodecInfo, android.util.Range<Int>> {
            ScreenFrameLimits.checkedBytes(config.width,config.height)
            require(config.fps in 1..240)
            val refusals = mutableListOf<String>()
            for (info in encoders().filter { supports(it, config.profile) }) {
                val caps = info.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC)
                val video = caps.videoCapabilities ?: continue
                val color=if(config.hybrid) MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible else MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface
                if(!caps.colorFormats.contains(color)) { refusals += "${info.name}: Eingabefarbformat nicht unterstützt"; continue }
                if (!video.areSizeAndRateSupported(config.width, config.height, config.fps.toDouble())) {
                    refusals += "${info.name}: ${config.width}x${config.height}@${config.fps} nicht unterstützt"
                    continue
                }
                val requestedProfile = profileOf(config.profile)
                val profile = if(config.profile=="cb" && caps.profileLevels.none {it.profile==requestedProfile})
                    MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline else requestedProfile
                val bitrate=config.bitrate.coerceIn(video.bitrateRange.lower,video.bitrateRange.upper)
                val supportedLevel = caps.profileLevels.filter { it.profile == profile }.sortedBy { it.level }.firstOrNull {
                    val level=MediaCodecInfo.CodecCapabilities.createFromProfileLevel(MediaFormat.MIMETYPE_VIDEO_AVC, profile, it.level)?.videoCapabilities
                    level != null && level.areSizeAndRateSupported(config.width,config.height,config.fps.toDouble()) && level.bitrateRange.contains(bitrate)
                }?.level
                if (supportedLevel == null) { refusals += "${info.name}: kein belegtes Profil/Level für Größe und Rate"; continue }
                val level = supportedLevel
                val levelRange=MediaCodecInfo.CodecCapabilities.createFromProfileLevel(MediaFormat.MIMETYPE_VIDEO_AVC,profile,level)?.videoCapabilities?.bitrateRange ?: continue
                val acceptedBitrate=android.util.Range(maxOf(video.bitrateRange.lower,levelRange.lower),minOf(video.bitrateRange.upper,levelRange.upper))
                val mode = listOf(MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR, MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_VBR)
                    .firstOrNull { caps.encoderCapabilities.isBitrateModeSupported(it) }

                if (mode == null) { refusals += "${info.name}: weder CBR noch VBR unterstützt"; continue }
                // Optional real-time hints are retried without them; the negotiated profile is never removed.
                for (realtime in listOf(true, false)) {
                    val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, config.width, config.height).apply {
                        setInteger(MediaFormat.KEY_COLOR_FORMAT, if(config.hybrid) MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible else MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
                        setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
                        setInteger(MediaFormat.KEY_BITRATE_MODE, mode)
                        setInteger(MediaFormat.KEY_FRAME_RATE, config.fps)
                        setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, KEYFRAME_INTERVAL_S)
                        setInteger(MediaFormat.KEY_PROFILE, profile)
                        setInteger(MediaFormat.KEY_LEVEL, level)
                        if(config.hybrid) ScreenCodecLatency.requestEncoder(this)
                        if (realtime) { setInteger(MediaFormat.KEY_PRIORITY, 0); setFloat(MediaFormat.KEY_OPERATING_RATE, config.fps.toFloat()) }
                        setInteger(MediaFormat.KEY_COLOR_STANDARD, MediaFormat.COLOR_STANDARD_BT709)
                        setInteger(MediaFormat.KEY_COLOR_RANGE, MediaFormat.COLOR_RANGE_LIMITED)
                        if(!config.hybrid) setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, REPEAT_AFTER_US)
                    }
                    if (!caps.isFormatSupported(format)) { refusals += "${info.name}: Format abgelehnt (Echtzeit=$realtime)"; continue }
                    try {
                        val candidate = construction.ownProducer(MediaCodec.createByCodecName(info.name))
                        candidate.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                        if(config.hybrid) ScreenCodecLatency.encoderFormat(info.name,candidate.outputFormat)
                        if(realtime) {
                            val accepted=candidate.inputFormat
                            require(accepted.containsKey(MediaFormat.KEY_PRIORITY) && accepted.getInteger(MediaFormat.KEY_PRIORITY)==0
                                && accepted.containsKey(MediaFormat.KEY_OPERATING_RATE)) { "Echtzeithinweise nicht bestätigt" }
                        }
                        Log.i(TAG, "Kodierer ${info.name}: ${config.width}x${config.height}@${config.fps}, Profil=$profile Level=$level Modus=$mode Echtzeit=$realtime")
                        return Triple(candidate,info,acceptedBitrate)
                    } catch (e: Throwable) {
                        try {construction.discardProducer()} catch(cleanup:Throwable) {
                            if(cleanup!==e) try {e.addSuppressed(cleanup)} catch(_:Throwable) {Unit}
                            throw e
                        }
                        if(e !is RuntimeException) throw e
                        refusals += "${info.name}: ${(e.message ?: e.javaClass.simpleName).take(512)} (Echtzeit=$realtime)"
                    }
                }
            }
            throw IllegalStateException("Kein passender H.264-Kodierer: ${refusals.joinToString("; ")}")
        }

        /** Name and hardware flag of the encoder [profile] would take, `null` without one. */
        fun best(profile: String = "high"): Pair<String, Boolean>? = pick(profile)?.let { it.name to it.isHardwareAccelerated }

        /** The offers this phone reports to the core (`screen.hostPrereqs`). */
        fun offers(): List<ScreenEncoderOffer> = buildList {
            for (profile in listOf("high", "cb")) {
                for(info in encoders().filter { supports(it,profile) }) {
                    val caps=info.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC)
                    val profiles=if(profile=="high") setOf(MediaCodecInfo.CodecProfileLevel.AVCProfileHigh)
                        else setOf(MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline,MediaCodecInfo.CodecProfileLevel.AVCProfileConstrainedBaseline)
                    for(limit in ScreenFrameLimits.offers(caps,profiles)) add(ScreenEncoderOffer(
                        profile,info.isHardwareAccelerated,limit.width,limit.height,limit.avc?.blocks,limit.avc?.blocksPerSecond))
                }
            }
        }.distinct().take(31)
    }
}
