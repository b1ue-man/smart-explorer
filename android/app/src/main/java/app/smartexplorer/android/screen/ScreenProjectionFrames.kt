package app.smartexplorer.android.screen

import android.graphics.PixelFormat
import android.media.Image
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface

/** Holds one newest projection image. Pixels are copied only for an admitted capture request. */
internal class ScreenProjectionFrames(
    private val width: Int,
    private val height: Int,
    private val submit: (Long, ByteArray) -> Boolean,
    private val failed: (String) -> Unit,
) {
    private val reader: ImageReader = run {
        require(width>0 && height>0 && width.toLong()*height*4*8<=512L*1024*1024)
        ImageReader.newInstance(width,height,PixelFormat.RGBA_8888,3)
    }
    private val thread=HandlerThread("screen-projection-pixels").also { it.start() }
    private val handler=Handler(thread.looper)
    private var latest: ByteArray?=null
    private var latestImage: Image?=null
    private var requested: Long?=null
    private var still: ((ByteArray) -> Boolean)?=null
    @Volatile private var closed=false
    val surface: Surface get()=reader.surface
    init {
        reader.setOnImageAvailableListener({ source ->
            if(!closed) try {
                source.acquireLatestImage()?.let { image -> latestImage?.close(); latestImage=image; sendRequested() }
            } catch(e: RuntimeException) { failed("RGBA-Capture: ${e.javaClass.simpleName}") }
        },handler)
    }
    fun capture(frame: Long) {
        if(closed || frame<=0) return
        handler.post {
            if(requested != null && requested != frame) { failed("Überlappende Captureanforderung"); return@post }
            requested=frame; sendRequested()
        }
    }
    fun captureStill(frame: Long, sink: (ByteArray) -> Boolean) {
        if(closed || frame<=0) return
        handler.post { requested=frame; still=sink; sendRequested() }
    }
    private fun pixels(image: Image): ByteArray {
        require(image.width==width && image.height==height && image.planes.size==1)
        val plane=image.planes[0]; val input=plane.buffer
        val bytes=ByteArray(Math.multiplyExact(Math.multiplyExact(width,height),4))
        for(y in 0 until height) for(x in 0 until width) {
            val at=input.position().toLong()+y.toLong()*plane.rowStride+x.toLong()*plane.pixelStride
            require(at>=0 && at+4<=input.limit())
            val out=(y*width+x)*4
            bytes[out]=input.get(at.toInt()); bytes[out+1]=input.get(at.toInt()+1)
            bytes[out+2]=input.get(at.toInt()+2); bytes[out+3]=255.toByte()
        }
        return bytes
    }
    private fun sendRequested() {
        val frame=requested ?: return
        try {
            latestImage?.let { image -> latestImage=null; image.use { latest=pixels(it) } }
            val pixels=latest ?: return
            if(still?.invoke(pixels) ?: submit(frame,pixels)) { requested=null; still=null }
            else failed("Kodierer nimmt zugelassenen Captureframe nicht an")
        } catch(e: RuntimeException) { requested=null; still=null; failed("RGBA-Kodierung: ${e.javaClass.simpleName}") }
    }
    fun close(): Boolean {
        if(closed) return !thread.isAlive
        closed=true; reader.setOnImageAvailableListener(null,null); handler.removeCallbacksAndMessages(null)
        handler.post { latestImage?.close(); latestImage=null; latest=null; requested=null; still=null; reader.close(); thread.quitSafely() }
        if(Thread.currentThread()!==thread) thread.join(2_000)
        return !thread.isAlive
    }
}
