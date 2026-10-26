package app.smartexplorer.android.screen

import android.graphics.PixelFormat
import android.media.Image
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface

/** Three acquired native images; the undocumented BufferQueue is not an exact product byte count. */
internal class ScreenProjectionFrames(
    private val width: Int,
    private val height: Int,
    private val budget: ScreenCaptureBudget,
    private val submit: (Long, ScreenCaptureBuffer) -> Boolean,
    private val failed: (String) -> Unit,
) {
    private val bytes=ScreenFrameLimits.checkedBytes(width,height)
    private val readerLease=budget.claim(ScreenCaptureBudget.NATIVE_SURFACE,Math.multiplyExact(bytes.toLong(),3))
    private val reader=try {ImageReader.newInstance(width,height,PixelFormat.RGBA_8888,3)}
        catch(error: Throwable) {readerLease.close();throw error}
    private val thread=HandlerThread("screen-projection-pixels").also {it.start()}
    private val handler=Handler(thread.looper)
    private var latest: ScreenCaptureBuffer?=null
    private var latestImage: Image?=null
    private var requested: Long?=null
    private var still: ((ScreenCaptureBuffer) -> Boolean)?=null
    @Volatile private var closed=false
    val surface: Surface get()=reader.surface
    init {
        reader.setOnImageAvailableListener({source ->
            if(!closed) try {
                source.acquireLatestImage()?.let {image ->
                    latestImage?.close();latestImage=image;sendRequested()
                }
            } catch(error: RuntimeException) {if(!closed) failed("RGBA-Capture: ${error.javaClass.simpleName}")}
        },handler)
    }
    fun capture(frame: Long) {
        if(closed || frame<=0) return
        handler.post {
            if(closed) return@post
            if(requested!=null && requested!=frame) {failed("Überlappende Captureanforderung");return@post}
            requested=frame;sendRequested()
        }
    }
    fun captureStill(frame: Long, sink: (ScreenCaptureBuffer) -> Boolean) {
        if(closed || frame<=0) return
        handler.post {
            if(closed) return@post
            if(requested!=null && requested!=frame) {failed("Überlappende Stillanforderung");return@post}
            requested=frame;still=sink;sendRequested()
        }
    }
    private fun pixels(image: Image): ScreenCaptureBuffer? {
        require(image.width==width && image.height==height && image.planes.size==1)
        // Admission precedes allocation. A fourth distinct RGBA buffer cannot
        // arise while an encoder, a previous image and the producer overlap.
        val output=budget.tryBuffer(ScreenCaptureBudget.SOURCE_PIXELS,bytes) ?: return null
        try {
            val plane=image.planes[0];val input=plane.buffer
            output.access {target ->
                for(y in 0 until height) for(x in 0 until width) {
                    val at=input.position().toLong()+y.toLong()*plane.rowStride+x.toLong()*plane.pixelStride
                    require(at>=0 && at+4<=input.limit())
                    target.put(input.get(at.toInt()));target.put(input.get(at.toInt()+1))
                    target.put(input.get(at.toInt()+2));target.put(255.toByte())
                }
            }
            return output
        } catch(error: Throwable) {output.close();throw error}
    }
    private fun sendRequested() {
        if(closed) return
        val frame=requested ?: return
        try {
            latestImage?.let {image ->
                val next=pixels(image) ?: return
                latestImage=null;image.close()
                val previous=latest;latest=next;previous?.close()
            }
            val pixels=latest ?: return
            if(closed) return
            if(still?.invoke(pixels) ?: submit(frame,pixels)) {requested=null;still=null}
            else if(!closed) failed("Kodierer nimmt zugelassenen Captureframe nicht an")
        } catch(error: RuntimeException) {
            requested=null;still=null
            if(!closed) failed("RGBA-Kodierung: ${error.javaClass.simpleName}")
        }
    }
    fun close(): Boolean {
        if(closed) return !thread.isAlive
        closed=true
        reader.setOnImageAvailableListener(null,null)
        handler.removeCallbacksAndMessages(null)
        // Running callbacks finish before this cleanup. A timed-out join does
        // not release their reader, pixels or native admission.
        handler.post {
            try {latestImage?.close();latestImage=null;latest?.close();latest=null;requested=null;still=null;reader.close()}
            finally {readerLease.close();thread.quitSafely()}
        }
        if(Thread.currentThread()!==thread) thread.join(2_000)
        return !thread.isAlive
    }
}
