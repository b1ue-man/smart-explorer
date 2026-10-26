package app.smartexplorer.android.screen

import android.media.MediaCodecInfo.CodecCapabilities
import android.media.MediaCodecInfo.CodecProfileLevel
import android.media.MediaFormat
import app.smartexplorer.android.api.ScreenBufferBudget

/** Absolute byte geometry; v2 also binds transition/record slots through ScreenBufferBudget. */
internal object ScreenFrameLimits {
    const val MAX_SIDE = 16384
    const val MAX_IMAGE_BYTES = 64 * 1024 * 1024
    /** Allocator fact observed when opening a session, before its frame buffers exist. */
    fun workingBytes(): Long {
        val runtime=Runtime.getRuntime()
        val used=(runtime.totalMemory()-runtime.freeMemory()).coerceAtLeast(0)
        return (runtime.maxMemory()-used).coerceIn(0,512L*1024*1024)
    }
    fun checkedBytes(width: Int, height: Int): Int {
        require(width in 1..MAX_SIDE && height in 1..MAX_SIDE)
        val bytes = Math.multiplyExact(Math.multiplyExact(width.toLong(), height.toLong()), 4L)
        require(bytes <= MAX_IMAGE_BYTES)
        return Math.toIntExact(bytes)
    }
    fun checkOutput(format: MediaFormat, width: Int, height: Int, budget:app.smartexplorer.android.api.ScreenBufferBudget?=null) {
        val codedWidth=format.getInteger(MediaFormat.KEY_WIDTH)
        val codedHeight=format.getInteger(MediaFormat.KEY_HEIGHT)
        checkedBytes(codedWidth,codedHeight)
        budget?.checkedBytes(codedWidth,codedHeight)
        require(codedWidth>=width && codedHeight>=height && codedWidth.toLong()<=width.toLong()+15 && codedHeight.toLong()<=height.toLong()+15)
        fun crop(key: String, fallback: Int) = if(format.containsKey(key)) format.getInteger(key) else fallback
        val left=crop("crop-left",0)
        val top=crop("crop-top",0)
        val right=crop("crop-right",codedWidth-1)
        val bottom=crop("crop-bottom",codedHeight-1)
        require(left>=0 && top>=0 && right<codedWidth && bottom<codedHeight)
        require(right.toLong()-left+1==width.toLong() && bottom.toLong()-top+1==height.toLong())
    }
    data class Avc(val blocks: Long, val blocksPerSecond: Long)
    data class Offer(val profile: Int, val width: Int, val height: Int, val avc: Avc?)

    // AVC Annex A, also represented by AOSP android-16.0.0_r1 VideoCapabilities.
    fun avc(level: Int): Avc? = when (level) {
        CodecProfileLevel.AVCLevel1, CodecProfileLevel.AVCLevel1b -> Avc(99,1485)
        CodecProfileLevel.AVCLevel11 -> Avc(396,3000)
        CodecProfileLevel.AVCLevel12 -> Avc(396,6000)
        CodecProfileLevel.AVCLevel13, CodecProfileLevel.AVCLevel2 -> Avc(396,11880)
        CodecProfileLevel.AVCLevel21 -> Avc(792,19800)
        CodecProfileLevel.AVCLevel22 -> Avc(1620,20250)
        CodecProfileLevel.AVCLevel3 -> Avc(1620,40500)
        CodecProfileLevel.AVCLevel31 -> Avc(3600,108000)
        CodecProfileLevel.AVCLevel32 -> Avc(5120,216000)
        CodecProfileLevel.AVCLevel4, CodecProfileLevel.AVCLevel41 -> Avc(8192,245760)
        CodecProfileLevel.AVCLevel42 -> Avc(8704,522240)
        CodecProfileLevel.AVCLevel5 -> Avc(22080,589824)
        CodecProfileLevel.AVCLevel51 -> Avc(36864,983040)
        CodecProfileLevel.AVCLevel52 -> Avc(36864,2073600)
        CodecProfileLevel.AVCLevel6 -> Avc(139264,4177920)
        CodecProfileLevel.AVCLevel61 -> Avc(139264,8355840)
        CodecProfileLevel.AVCLevel62 -> Avc(139264,16711680)
        else -> null
    }
    fun offers(caps: CodecCapabilities, profiles: Set<Int>): List<Offer> {
        val actual = caps.videoCapabilities ?: return emptyList()
        return profiles.mapNotNull { profile ->
            val supported = caps.profileLevels.filter { it.profile == profile }
            if (supported.isEmpty()) return@mapNotNull null
            val best = supported.filter { avc(it.level) != null }.maxWithOrNull(
                compareBy({ avc(it.level)!!.blocks }, { avc(it.level)!!.blocksPerSecond }))
            val standard = best?.let { CodecCapabilities.createFromProfileLevel("video/avc",profile,it.level)?.videoCapabilities }
            Offer(profile,
                minOf(actual.supportedWidths.upper,standard?.supportedWidths?.upper ?: 4096,MAX_SIDE),
                minOf(actual.supportedHeights.upper,standard?.supportedHeights?.upper ?: 2304,MAX_SIDE),
                best?.let { avc(it.level) })
        }.filter { it.width > 0 && it.height > 0 }
    }

    fun checkAvc(data: java.nio.ByteBuffer, offset: Int, length: Int, width: Int, height: Int, budget: ScreenBufferBudget?) {
        checkedBytes(width,height)
        budget?.checkedBytes(width,height)
        require(offset>=0 && length>=0 && offset.toLong()+length<=data.capacity())
        var at=offset
        val end=offset+length
        while(at+3<end) {
            val prefix=data.get(at)==0.toByte() && data.get(at+1)==0.toByte() && data.get(at+2)==1.toByte()
            if(prefix) require(data.get(at+3).toInt() and 31!=15)
            if(prefix && data.get(at+3).toInt() and 31==7) {
                var next=at+4
                while(next+3<end && !(data.get(next)==0.toByte() && data.get(next+1)==0.toByte() && data.get(next+2)==1.toByte())) next++
                if(next+3>=end) next=end
                require(next-at<=65540)
                val bytes=ByteArray(next-at)
                data.duplicate().apply {position(at);limit(next)}.get(bytes)
                checkAvc(bytes,width,height,budget)
                at=next
            } else at++
        }
    }

    /** In-band SPS is checked before MediaCodec sees any new coded dimensions. */
    fun checkAvc(data: ByteArray, width: Int, height: Int, budget: ScreenBufferBudget?) {
        checkedBytes(width,height)
        budget?.checkedBytes(width,height)
        fun start(at: Int) = at+2 < data.size && data[at]==0.toByte() && data[at+1]==0.toByte() && data[at+2]==1.toByte()
        var at=0
        while (at+3 < data.size) {
            if (!start(at)) { at++; continue }
            val from=at+3
            at=from+1
            while (at+3 < data.size && !start(at)) at++
            val end=if (at+3 < data.size) at else data.size
            require(data[from].toInt() and 31 != 15)
            if (data[from].toInt() and 31 != 7) continue
            require(end-from <= 65536)
            val raw=ByteArray(end-from-1)
            var count=0
            var zeros=0
            for (i in from+1 until end) {
                val byte=data[i].toInt() and 255
                if (zeros >= 2 && byte == 3) {
                    require(i+1<end && (data[i+1].toInt() and 255)<=3)
                    zeros=0; continue
                }
                raw[count++]=data[i]
                zeros=if (byte==0) zeros+1 else 0
            }
            val bits=Bits(raw,count)
            val profile=bits.read(8)
            val constraints=bits.read(8)
            val signalledLevel=bits.read(8)
            val level=if(signalledLevel==11 && constraints and 0x10!=0 && (profile==66 || profile==77)) 9 else signalledLevel
            require(bits.ue()<=31)
            if (profile==100) {
                require(bits.ue()==1 && bits.ue()==0 && bits.ue()==0)
                bits.read(1)
                if (bits.read(1)!=0) repeat(8) { i ->
                    if (bits.read(1)!=0) {
                        var last=8L
                        var next=8L
                        repeat(if(i<6)16 else 64) {
                            if (next!=0L) next=Math.floorMod(last+bits.se()+256,256L)
                            if (next!=0L) last=next
                        }
                    }
                }
            } else require(profile==66 || profile==77)
            require(bits.ue()<=12)
            when (bits.ue()) {
                0 -> require(bits.ue()<=12)
                1 -> {
                    bits.read(1); bits.se(); bits.se()
                    val cycle=bits.ue()
                    require(cycle<=256)
                    repeat(cycle) { bits.se() }
                }
                2 -> Unit
                else -> error("Invalid AVC picture order type")
            }
            require(bits.ue()<=16)
            bits.read(1)
            val codedWidth=Math.multiplyExact(Math.addExact(bits.ue(),1),16)
            val codedHeight=Math.multiplyExact(Math.addExact(bits.ue(),1),16)
            require(bits.read(1)==1)
            bits.read(1)
            val crop=IntArray(4)
            if (bits.read(1)!=0) for(i in crop.indices) crop[i]=bits.ue()
            val cropX=Math.multiplyExact(Math.addExact(crop[0],crop[1]),2)
            val cropY=Math.multiplyExact(Math.addExact(crop[2],crop[3]),2)
            require(codedWidth.toLong()-cropX==width.toLong() && codedHeight.toLong()-cropY==height.toLong())
            checkedBytes(codedWidth,codedHeight)
            budget?.checkedBytes(codedWidth,codedHeight)
            require(codedWidth.toLong()<=width.toLong()+15 && codedHeight.toLong()<=height.toLong()+15)
            val maxBlocks=when(level) {
                9,10 -> 99; 11,12,13,20 -> 396; 21 -> 792; 22,30 -> 1620
                31 -> 3600; 32 -> 5120; 40,41 -> 8192; 42 -> 8704
                50 -> 22080; 51,52 -> 36864; 60,61,62 -> 139264
                else -> error("Unknown AVC level")
            }
            val blocks=codedWidth.toLong()/16*(codedHeight/16)
            val side=kotlin.math.sqrt(maxBlocks.toDouble()*8).toLong()
            require(blocks<=maxBlocks && codedWidth/16<=side && codedHeight/16<=side)
        }
    }
    private class Bits(private val data: ByteArray, private val size: Int) {
        private var at=0
        fun read(count: Int): Int {
            require(count in 0..31 && at.toLong()+count<=size.toLong()*8)
            var result=0
            repeat(count) { result=(result shl 1) or ((data[at/8].toInt() ushr (7-at%8)) and 1); at++ }
            return result
        }
        fun ue(): Int {
            var zeros=0
            while(read(1)==0) { zeros++; require(zeros<=30) }
            return (1 shl zeros)-1+read(zeros)
        }
        fun se(): Long { val n=ue().toLong(); return if(n and 1L==0L)-n/2 else (n+1)/2 }
    }
}
