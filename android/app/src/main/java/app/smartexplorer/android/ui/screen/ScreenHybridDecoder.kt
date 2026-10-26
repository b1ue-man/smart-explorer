package app.smartexplorer.android.ui.screen

import android.graphics.Bitmap
import app.smartexplorer.android.api.ScreenBufferBudget
import app.smartexplorer.android.screen.ScreenFrameLimits
import app.smartexplorer.android.screen.ScreenCodecLatency
import android.util.Log
import java.util.concurrent.CancellationException
import android.media.Image
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** V2 composite owns one confirmed frame; staging pixels are never presented. */
internal class ScreenHybridDecoder(private val width: Int, private val height: Int,
    private val budget:ScreenBufferBudget=ScreenBufferBudget.LEGACY, private val isRunning: () -> Boolean = {true}) {
    private data class Stamp(val generation: Long, val frame: Long, val base: Long)
    private data class Rect(val x: Int, val y: Int, val w: Int, val h: Int)
    private val bytes = budget.checkedBytes(width, height)
    private var picture = ByteArray(bytes)
    private var spare=ByteArray(bytes)
    private var bitmap:Bitmap?=null
    private var staging: ByteArray? = null
    private var active: Stamp? = null
    private var confirmed: Stamp? = null
    private var key = false
    private var order = 0
    private var operations = 0
    private var encodedBytes=0L
    private var covered = BooleanArray(((width + 63) / 64) * ((height + 63) / 64))
    private var codec: MediaCodec? = null
    private var videoValid = false
    private var codecName:String?=null
    private var videoFormat: MediaFormat? = null
    val completeFrameId: Long? get() = confirmed?.frame

    fun close() {
        codec?.let(::disposeCodec)
        codec=null;codecName=null;videoFormat=null
        videoValid = false
        staging = null
        active = null
        confirmed = null
    }

    fun accept(data: ByteArray, keyframe: Boolean, more: Boolean): Bitmap? {
        require(data.size in 25..budget.recordBytes)
        val input = ByteBuffer.wrap(data).order(ByteOrder.BIG_ENDIAN)
        val stamp = Stamp(input.long, input.long, input.long)
        require(stamp.generation > 0 && stamp.frame > 0)
        when (input.get().toInt()) {
            0 -> {
                require(active == null && more && !input.hasRemaining())
                val last = confirmed
                if (keyframe) require(stamp.base == 0L && (last == null || stamp.frame > last.frame && stamp.generation >= last.generation))
                else require(last != null && last.generation == stamp.generation && last.frame == stamp.base && stamp.frame > last.frame)
                active = stamp
                key = keyframe
                staging=spare
                if(keyframe) spare.fill(0) else picture.copyInto(spare)
                encodedBytes=data.size.toLong()
                covered.fill(false)
                order = 0
                operations = 0
                return null
            }
        }
        require(active == stamp && key == keyframe)
        encodedBytes+=data.size
        require(encodedBytes<=budget.packetBytes)
        require(++operations <= covered.size * 3 + 4)
        input.position(24)
        when (input.get().toInt()) {
            1 -> {
                require(order == 0)
                val videoKey = when (input.get().toInt()) { 0 -> false; 1 -> true; else -> error("Invalid video key flag") }
                require(videoValid || videoKey)
                val count = input.int
                require(count in 1..covered.size && count <= input.remaining() / 16)
                val rects = List(count) { rect(input) }
                val encoded = ByteArray(input.remaining()).also { input.get(it) }
                decodeVideo(encoded,stamp.frame,videoKey,rects)
                rects.forEach(::cover)
                videoValid = true
                order = 1
            }
            2 -> {
                require(!key && order <= 1)
                val count = input.int
                require(count in 0..covered.size*2 && count <= input.remaining() / 24)
                repeat(count) {
                    val x = input.int
                    val y = input.int
                    val dst = rect(input)
                    val src = Rect(x, y, dst.w, dst.h).also(::validate)
                    copy(picture, src, staging!!, dst)
                }
                order = 2
            }
            3 -> {
                val r = rect(input)
                require(r.w <= 64 && r.h <= 64 && input.remaining() <= 64 * 64 * 4 + 128)
                val compressed = ByteArray(input.remaining()).also { input.get(it) }
                val raw = lz4(compressed, r.w * r.h * 4)
                // Lossless wire pixels are B,G,R,A; Android bitmap bytes are R,G,B,A.
                for (at in raw.indices step 4) { val b = raw[at]; raw[at] = raw[at+2]; raw[at+2] = b }
                for (row in 0 until r.h) raw.copyInto(staging!!, ((r.y+row)*width+r.x)*4, row*r.w*4, (row+1)*r.w*4)
                cover(r)
                order = 3
            }
            4 -> {
                require(!more && !input.hasRemaining() && (!key || covered.all { it }))
                val previous=picture
                picture = staging ?: error("Missing composite")
                spare=previous
                staging = null
                confirmed = stamp
                active = null
                val target=bitmap ?: Bitmap.createBitmap(width,height,Bitmap.Config.ARGB_8888).also {bitmap=it}
                return target.also {
                    it.copyPixelsFromBuffer(ByteBuffer.wrap(picture))
                }
            }
            else -> error("Unknown hybrid operation")
        }
        require(more && !input.hasRemaining())
        return null
    }

    private fun rect(input: ByteBuffer): Rect = Rect(input.int,input.int,input.int,input.int).also(::validate)
    private fun validate(r: Rect) {
        require(r.x >= 0 && r.y >= 0 && r.w > 0 && r.h > 0 && r.x.toLong()+r.w <= width && r.y.toLong()+r.h <= height)
    }
    private fun cover(r: Rect) {
        val columns = (width+63)/64
        for (y in r.y/64..(r.y+r.h-1)/64) for (x in r.x/64..(r.x+r.w-1)/64) {
            val left=x*64; val top=y*64
            if (r.x <= left && r.y <= top && r.x+r.w >= minOf(left+64,width) && r.y+r.h >= minOf(top+64,height)) covered[y*columns+x]=true
        }
    }
    private fun copy(src: ByteArray, from: Rect, dst: ByteArray, to: Rect) {
        validate(from); validate(to)
        for (row in 0 until from.h) {
            val offset=((from.y+row)*width+from.x)*4
            src.copyInto(dst,((to.y+row)*width+to.x)*4,offset,offset+from.w*4)
        }
    }
    private fun disposeCodec(value: MediaCodec) {
        try {value.stop()} catch (_: RuntimeException) {Unit}
        try {value.release()} catch (e: RuntimeException) {Log.w("ScreenHybridDecoder","Codecfreigabe: ${e.javaClass.simpleName}")}
    }
    private fun decodeVideo(data: ByteArray, frame: Long, keyframe: Boolean, rects:List<Rect>) {
        ScreenFrameLimits.checkAvc(data,width,height,budget)
        val deadline=System.nanoTime()+ScreenCodecLatency.TRANSACTION_NS
        val current=codec
        if(current!=null) {
            try {
                decodeOne(current,data,frame,deadline,codecName ?: "MediaCodec",rects)
                return
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (e: RuntimeException) {
                disposeCodec(current);codec=null
                throw ScreenCodecLatency.Unavailable(codecName ?: "MediaCodec",e.message ?: "Einzelinput-Ausgabe unbestätigt")
            }
        }
        require(keyframe)
        var profile:Int?=null
        for(at in 0 until (data.size-4).coerceAtLeast(0)) {
            if(data[at]==0.toByte() && data[at+1]==0.toByte() && data[at+2]==1.toByte() && data[at+3].toInt() and 31==7) {profile=data[at+4].toInt() and 255;break}
        }
        val refused=mutableListOf<String>()
        val candidates=MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.mapNotNull {info ->
            try {
                if(info.isEncoder || info.isAlias || info.supportedTypes.none {it.equals("video/avc",true)}) return@mapNotNull null
                val caps=info.getCapabilitiesForType("video/avc")
                if(profile==100 && caps.profileLevels.none {it.profile==MediaCodecInfo.CodecProfileLevel.AVCProfileHigh}) return@mapNotNull null
                if(!caps.colorFormats.contains(MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible)) return@mapNotNull null
                val format=MediaFormat.createVideoFormat("video/avc",width,height).apply {
                    setInteger(MediaFormat.KEY_COLOR_FORMAT,MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible)
                    setInteger(MediaFormat.KEY_MAX_INPUT_SIZE,budget.recordBytes)
                }
                val requested=ScreenCodecLatency.requestDecoder(caps,format)
                if(!caps.isFormatSupported(format)) null else Triple(info,format,requested)
            } catch(e: RuntimeException) {refused+="${info.name}: Fähigkeiten unbestätigt (${e.javaClass.simpleName})";null}
        }.sortedWith(compareByDescending<Triple<MediaCodecInfo,MediaFormat,Boolean>> {it.third}.thenByDescending {it.first.isHardwareAccelerated})
        for((position,candidate) in candidates.withIndex()) {
            if(!isRunning()) throw CancellationException()
            val remaining=deadline-System.nanoTime()
            if(remaining<=0) break
            val candidateDeadline=System.nanoTime()+remaining/(candidates.size-position)
            val (info,format,requested)=candidate
            var opened:MediaCodec?=null
            try {
                opened=MediaCodec.createByCodecName(info.name)
                opened.configure(format,null,null,0)
                val confirmed=ScreenCodecLatency.checkDecoderHint(opened.inputFormat,requested)
                Log.i("ScreenHybridDecoder","${info.name}: LowLatency angefragt=$requested im Format bestätigt=$confirmed; Einzelinput-Ausgabe noch unbestätigt")
                opened.start()
                videoFormat=null
                decodeOne(opened,data,frame,candidateDeadline,info.name,rects)
                codec=opened;codecName=info.name
                return
            } catch(cancelled: CancellationException) {
                opened?.let(::disposeCodec);throw cancelled
            } catch(e: RuntimeException) {
                refused+="${info.name}: ${e.message ?: e.javaClass.simpleName}"
                opened?.let(::disposeCodec)
            }
        }
        throw ScreenCodecLatency.Unavailable("Android-Hybriddecoder",
            "kein passendes zugängliches Einzelinputbild innerhalb der Referenzfrist; ${refused.joinToString("; ").take(512)}")
    }
    private fun decodeOne(c:MediaCodec,data:ByteArray,frame:Long,deadline:Long,name:String,rects:List<Rect>) {
        var index=-1
        while(index<0 && System.nanoTime()<deadline) {
            if(!isRunning()) throw CancellationException()
            index=c.dequeueInputBuffer(ScreenCodecLatency.POLL_US)
        }
        if(index<0) throw ScreenCodecLatency.Unavailable(name,"kein einzelner Eingabepuffer innerhalb der Referenzfrist")
        val input=c.getInputBuffer(index) ?: throw ScreenCodecLatency.Unavailable(name,"kein zugänglicher Eingabepuffer")
        if(data.size>input.capacity()) throw ScreenCodecLatency.Unavailable(name,"AU überschreitet den Codec-Eingabepuffer")
        input.clear();input.put(data);c.queueInputBuffer(index,0,data.size,frame,0)
        val info=MediaCodec.BufferInfo()
        while(System.nanoTime()<deadline) {
            if(!isRunning()) throw CancellationException()
            val output=c.dequeueOutputBuffer(info,ScreenCodecLatency.POLL_US)
            if(output==MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                val format=c.outputFormat;ScreenFrameLimits.checkOutput(format,width,height,budget);videoFormat=format;continue
            }
            if(output<0) continue
            try {
                if(info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG!=0) continue
                if(info.presentationTimeUs!=frame) throw ScreenCodecLatency.Unavailable(name,"Decoder-Ausgabe widerspricht dem echten FrameStamp")
                val image=c.getOutputImage(output) ?: throw ScreenCodecLatency.Unavailable(name,"Decoder stellt keine Videopixel bereit")
                val format=videoFormat ?: c.outputFormat
                ScreenFrameLimits.checkOutput(format,width,height,budget)
                image.use {rgba(it,format,rects)}
                return
            } finally {c.releaseOutputBuffer(output,false)}
        }
        throw ScreenCodecLatency.Unavailable(name,"keine passende Ausgabe nach genau einer AU innerhalb der Referenzfrist")
    }
    private fun rgba(image: Image, format: MediaFormat, rects:List<Rect>) {
        val crop=image.cropRect
        require(crop.width()==width && crop.height()==height && image.planes.size==3)
        val planes=image.planes
        fun sample(p: Int,x: Int,y: Int): Int {
            val plane=planes[p]; val buffer=plane.buffer
            val at=buffer.position().toLong()+y.toLong()*plane.rowStride+x.toLong()*plane.pixelStride
            require(at >= 0 && at < buffer.limit())
            return buffer.get(at.toInt()).toInt() and 255
        }
        val full=format.containsKey(MediaFormat.KEY_COLOR_RANGE) && format.getInteger(MediaFormat.KEY_COLOR_RANGE)==MediaFormat.COLOR_RANGE_FULL
        val hd=!format.containsKey(MediaFormat.KEY_COLOR_STANDARD) || format.getInteger(MediaFormat.KEY_COLOR_STANDARD)==MediaFormat.COLOR_STANDARD_BT709
        val out=staging ?: error("Missing hybrid staging pixels")
        for(r in rects) for (y in r.y until r.y+r.h) for (x in r.x until r.x+r.w) {
            val sx=x+crop.left; val sy=y+crop.top
            val luma=sample(0,sx,sy); val u=sample(1,sx/2,sy/2)-128; val v=sample(2,sx/2,sy/2)-128
            val yy=if(full) luma.toDouble() else (luma-16)*255.0/219.0
            val uv=if(full) 1.0 else 255.0/224.0
            val at=(y*width+x)*4
            out[at]=(yy+(if(hd)1.5748 else 1.402)*v*uv).toInt().coerceIn(0,255).toByte()
            out[at+1]=(yy-(if(hd)0.187324 else 0.344136)*u*uv-(if(hd)0.468124 else 0.714136)*v*uv).toInt().coerceIn(0,255).toByte()
            out[at+2]=(yy+(if(hd)1.8556 else 1.772)*u*uv).toInt().coerceIn(0,255).toByte()
            out[at+3]=255.toByte()
        }
    }
    private fun lz4(input: ByteArray, size: Int): ByteArray {
        val out=ByteArray(size); var src=0; var dst=0
        fun length(start: Int): Int {
            var n=start
            if(start==15) { do { require(src<input.size); val extra=input[src++].toInt() and 255; n=Math.addExact(n,extra) } while(extra==255) }
            return n
        }
        while(src<input.size) {
            val token=input[src++].toInt() and 255
            val literals=length(token ushr 4)
            require(literals<=input.size-src && literals<=size-dst)
            input.copyInto(out,dst,src,src+literals); src+=literals; dst+=literals
            if(src==input.size) break
            require(src+2<=input.size)
            val offset=(input[src].toInt() and 255) or ((input[src+1].toInt() and 255) shl 8); src+=2
            require(offset>0 && offset<=dst)
            val match=Math.addExact(length(token and 15),4)
            require(match<=size-dst)
            repeat(match) { out[dst]=out[dst-offset]; dst++ }
        }
        require(dst==size)
        return out
    }
    companion object {
        const val MAX_RECORD=16*1024*1024
        fun checkedBytes(width: Int,height: Int): Int {
            return ScreenFrameLimits.checkedBytes(width,height)
        }
    }
}
