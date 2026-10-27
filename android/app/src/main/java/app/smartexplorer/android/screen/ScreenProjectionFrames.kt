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
    private val construction=ScreenCaptureConstruction<HandlerThread,ImageReader>(::retireProjectionThread,{it.close()})
    private val readerLease:ScreenCaptureLease
    private val reader:ImageReader
    private val thread:HandlerThread
    private val handler:Handler
    private var latest: ScreenCaptureBuffer?=null
    private var latestImage: Image?=null
    private var requested: Long?=null
    private var still: ((ScreenCaptureBuffer) -> Boolean)?=null
    @Volatile private var closed=false
    @Volatile private var resourcesReleased=false
    val surface: Surface get() {check(!closed) {"Projectionquelle ist bereits beendet"};return reader.surface}
    init {
        try {
            readerLease=budget.claim(ScreenCaptureBudget.NATIVE_SURFACE,Math.multiplyExact(bytes.toLong(),3))
            construction.ownClaim(readerLease)
            reader=checkNotNull(construction.ownSurface(ImageReader.newInstance(width,height,PixelFormat.RGBA_8888,3)))
            thread=construction.ownProducer(object : HandlerThread("screen-projection-pixels") {
                override fun run() {
                    try {super.run()} finally {
                        if(construction.producerEnded()) {closed=true;finishResources()}
                    }
                }
            })
            thread.start()
            handler=Handler(thread.looper)
            reader.setOnImageAvailableListener({source ->
                if(!closed) try {
                    source.acquireLatestImage()?.let {image ->
                        latestImage?.close();latestImage=image;sendRequested()
                    }
                } catch(error: RuntimeException) {if(!closed) failed("RGBA-Capture: ${error.javaClass.simpleName}")}
            },handler)
            check(construction.enterRuntime()) {"Projectionthread endete während seiner Konstruktion"}
        } catch(failure:Throwable) {closed=true;construction.abort(failure)}
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
            val plane=image.planes[0]
            // Row-wise bulk copy; a per-pixel loop cost millions of index calls per frame.
            output.access {target -> ScreenRgbaPlanes.copyRgba(ScreenPlane(plane.buffer,plane.rowStride,plane.pixelStride),width,height,target)}
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
        closed=true
        try {handler.removeCallbacksAndMessages(null)} finally {thread.quitSafely()}
        if(Thread.currentThread()!==thread) thread.join(2_000)
        // The worker's finally owns cleanup even after spontaneous failure or
        // a timed-out close. A dead worker also permits a failed cleanup retry.
        if(!thread.isAlive) finishResources()
        return !thread.isAlive && resourcesReleased
    }
    @Synchronized private fun finishResources() {
        if(resourcesReleased) return
        try {latestImage?.close();latestImage=null} finally {
            try {latest?.close();latest=null} finally {
                requested=null;still=null
                reader.close()
                readerLease.close()
                resourcesReleased=true
            }
        }
    }
}

/** A failed constructor has no caller that could stop its already started handler. */
private fun retireProjectionThread(worker: HandlerThread) {
    worker.quitSafely()
    var interrupted=false
    while(worker.isAlive) {
        try {worker.join()} catch(_:InterruptedException) {interrupted=true}
    }
    if(interrupted) Thread.currentThread().interrupt()
}
