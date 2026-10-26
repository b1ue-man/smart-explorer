package app.smartexplorer.android.ui.screen

import android.graphics.Bitmap
import app.smartexplorer.android.api.ScreenBufferBudget
import androidx.compose.foundation.Image
import androidx.compose.material3.Text
import androidx.compose.ui.graphics.Color
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first

internal object ScreenScaling {
    const val COPY_ERROR = "Bildkopie nicht verfügbar: zu wenig Darstellungsspeicher."
    fun snapshot(source:Bitmap,budget:ScreenBufferBudget?=ScreenBufferBudget.LEGACY):Bitmap? {
        return try {
            val actual=budget ?: return null
            actual.checkedBytes(source.width,source.height)
            require(!source.isRecycled && source.allocationByteCount<=actual.imageBytes)
            source.copy(Bitmap.Config.ARGB_8888,false)
        } catch(_:OutOfMemoryError) {
            null
        } catch(_:IllegalArgumentException) {
            null
        }
    }
    private val observedLimit=MutableStateFlow<Pair<Int,Int>?>(null)
    /** Actual Canvas limit. Source opening waits for the first draw, without blocking it. */
    fun rendererLimit():Pair<Int,Int>?=observedLimit.value
    suspend fun awaitRendererLimit():Pair<Int,Int> = observedLimit.filterNotNull().first()
    fun observeRenderer(canvas:android.graphics.Canvas) {
        val width=canvas.maximumBitmapWidth;val height=canvas.maximumBitmapHeight
        if(width>0 && height>0) observedLimit.value=minOf(width,16384) to minOf(height,16384)
    }
}

private class ScreenScaleOffer {
    var frame:Int?=null
    var key:List<Int>?=null
}

/** The session worker filters only visible destination pixels. Its retained
 * output leases survive composition until replacement/disposal; Full and Preview
 * cannot each create an independent resampling scratch pool. */
@Composable
internal fun ScreenBitmap(source:Bitmap,frame:Int,width:Int,height:Int,modifier:Modifier,
    viewportWidth:Int=width,viewportHeight:Int=height,left:Int=0,top:Int=0,identity:ScreenPresentedFrame?=null,
    scaler:ScreenScaleWorker,onPresented:(ScreenPresentedFrame)->Unit={},onRendered:(Boolean)->Unit={}) {
    val x=maxOf(0,left)
    val y=maxOf(0,top)
    val right=minOf(viewportWidth.toLong(),left.toLong()+width).toInt()
    val bottom=minOf(viewportHeight.toLong(),top.toLong()+height).toInt()
    if(right<=x || bottom<=y) return
    val key=listOf(width,height,x-left,y-top,right-x,bottom-y)
    val currentKey by rememberUpdatedState(key)
    val owner=remember(scaler) {scaler.owner()}
    val held=remember(owner) {ArrayList<ScreenScaled>()}
    val offered=remember(owner) {ScreenScaleOffer()}
    var scaled by remember(owner) {mutableStateOf<ScreenScaled?>(null)}
    var failed by remember(owner) {mutableStateOf<List<Int>?>(null)}
    DisposableEffect(owner) {
        onDispose {
            scaler.detach(owner)
            held.forEach {it.close()};held.clear()
        }
    }
    val selected=scaled?.takeIf {it.key==key}
    val ready=selected?.bitmap
    SideEffect {
        // This composition has replaced the previous draw content. A published
        // result is acknowledged only here, bounding waiting output to one.
        val keep=scaled
        held.removeAll {old->if(old!==keep) {old.close();true} else false}
        keep?.applied?.complete(Unit)
        if(offered.frame!=frame || offered.key!=key) {
            offered.frame=frame;offered.key=key
            scaler.submit(owner,source,key,identity) {result->
                if(currentKey!=key) false else {
                    if(result==null) {scaled=null;failed=key}
                    else {held.add(result);scaled=result;failed=null}
                    true
                }
            }
        }
        onRendered(ready!=null && failed!=key)
    }
    val density=LocalDensity.current
    val placed=modifier.offset {IntOffset(x,y)}.size(with(density) {(right-x).toDp()},with(density) {(bottom-y).toDp()})
    if(failed==key) Text("Bildskalierung nicht verfügbar: Renderer- oder Speicherressource fehlt.",color=Color.White,modifier=placed)
    else if(ready!=null) Image(ready.asImageBitmap(),contentDescription=null,modifier=placed.drawWithContent {
        drawContent()
        selected?.identity?.let(onPresented)
    },contentScale=ContentScale.FillBounds,filterQuality=FilterQuality.None)
}
