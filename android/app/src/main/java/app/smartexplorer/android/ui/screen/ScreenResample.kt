package app.smartexplorer.android.ui.screen

import android.graphics.Bitmap
import app.smartexplorer.android.api.ScreenBufferBudget
import app.smartexplorer.android.screen.ScreenFrameLimits
import kotlin.math.ceil
import kotlin.math.floor

/** The existing encoded-signal area/Catmull-Rom filter, with no per-pixel tap
 * objects and a fixed number of reusable horizontal/vertical rows. */
internal object ScreenResample {
    data class Plan(val width:Int,val height:Int,val x:Int,val y:Int,
        val outputWidth:Int,val outputHeight:Int,val sourceBytes:Int,val outputBytes:Int,
        val cachedRows:Int,val rowBytes:Long,val nativePixels:Boolean,val alias:Boolean) {
        // The previous Compose output keeps its own lease until replacement.
        val retainedBytes:Long get()=outputBytes.toLong()+1024
        // During createBitmap both the target IntArray and bitmap exist.
        val workingBytes:Long get()=if(alias) retainedBytes else
            (if(nativePixels) 0L else sourceBytes.toLong())+2L*outputBytes+rowBytes+1024
    }

    fun plan(source:Bitmap,width:Int,height:Int,x:Int,y:Int,outputWidth:Int,outputHeight:Int,
        budget:ScreenBufferBudget):Plan {
        require(width>0 && height>0 && x>=0 && y>=0)
        val sourceBytes=budget.checkedBytes(source.width,source.height)
        require(!source.isRecycled && source.allocationByteCount<=budget.imageBytes)
        val outputBytes=ScreenFrameLimits.checkedBytes(outputWidth,outputHeight)
        require(x.toLong()+outputWidth<=width && y.toLong()+outputHeight<=height)
        ScreenScaling.rendererLimit()?.let {require(outputWidth<=it.first && outputHeight<=it.second)}
        val nativePixels=source.width==width && source.height==height
        val alias=nativePixels && x==0 && y==0 && outputWidth==width && outputHeight==height
        val cached=if(height>source.height) minOf(source.height,4) else 1
        val rows=if(nativePixels) 0L else Math.multiplyExact(outputWidth.toLong()*12,cached+1L)
        return Plan(width,height,x,y,outputWidth,outputHeight,sourceBytes,outputBytes,cached,rows,nativePixels,alias)
    }

    private inline fun weights(source:Int,target:Int,pixel:Int,emit:(Int,Float)->Unit) {
        val scale=source.toDouble()/target
        if(scale>=1.0) {
            val lo=pixel*scale;val hi=(pixel+1.0)*scale
            for(i in floor(lo).toInt() until ceil(hi).toInt().coerceAtMost(source)) {
                emit(i,((minOf(hi,i+1.0)-maxOf(lo,i.toDouble()))/scale).toFloat())
            }
        } else {
            val center=(pixel+0.5)*scale-0.5;val first=floor(center).toInt()
            for(i in first-1..first+2) {
                val x=kotlin.math.abs(center-i)
                val w=if(x<1) 1.5*x*x*x-2.5*x*x+1 else if(x<2) -0.5*x*x*x+2.5*x*x-4*x+2 else 0.0
                emit(i.coerceIn(0,source-1),w.toFloat())
            }
        }
    }
    private fun encoded(v:Float)=(v.coerceIn(0f,1f)*255f+0.5f).toInt()

    /** Caller owns a workingBytes lease before entering this function. */
    fun resize(source:Bitmap,p:Plan,checkActive:()->Unit):Bitmap {
        checkActive()
        if(p.alias) return source
        if(p.nativePixels) {
            // Pixel-aligned native zoom is an exact crop: no tap/float buffers
            // and no full source IntArray, including a one-pixel-high source.
            val output=IntArray(p.outputBytes/4)
            source.getPixels(output,0,p.outputWidth,p.x,p.y,p.outputWidth,p.outputHeight)
            for(i in output.indices) {
                if(i%1024==0) checkActive()
                output[i]=output[i] or (255 shl 24)
            }
            checkActive()
            return Bitmap.createBitmap(output,p.outputWidth,p.outputHeight,Bitmap.Config.ARGB_8888)
        }
        val sw=source.width;val sh=source.height
        val src=IntArray(p.sourceBytes/4)
        source.getPixels(src,0,sw,0,0,sw,sh)
        val output=IntArray(p.outputBytes/4)
        val rowSize=Math.multiplyExact(p.outputWidth,3)
        val horizontal=Array(p.cachedRows) {FloatArray(rowSize)}
        val keys=IntArray(p.cachedRows) {-1}
        val row=FloatArray(rowSize)
        var next=0
        fun horizontalAt(sy:Int):FloatArray {
            val found=if(p.height>sh) keys.indexOf(sy) else -1
            if(found>=0) return horizontal[found]
            val slot=next;next=(next+1)%p.cachedRows;keys[slot]=sy
            val h=horizontal[slot];h.fill(0f)
            for(x in 0 until p.outputWidth) {
                if(x%256==0) checkActive()
                weights(sw,p.width,x+p.x) {sx,weight->
                    if(sx%256==0) checkActive()
                    val pixel=src[sy*sw+sx]
                    for(c in 0..2) h[x*3+c]+=((pixel ushr (16-c*8)) and 255)/255f*weight
                }
            }
            return h
        }
        for(y in 0 until p.outputHeight) {
            checkActive();row.fill(0f)
            weights(sh,p.height,y+p.y) {sy,weight->
                checkActive()
                val h=horizontalAt(sy)
                for(i in row.indices) row[i]+=h[i]*weight
            }
            for(x in 0 until p.outputWidth) {
                output[y*p.outputWidth+x]=(255 shl 24) or (encoded(row[x*3]) shl 16) or
                    (encoded(row[x*3+1]) shl 8) or encoded(row[x*3+2])
            }
        }
        checkActive()
        return Bitmap.createBitmap(output,p.outputWidth,p.outputHeight,Bitmap.Config.ARGB_8888)
    }
}
