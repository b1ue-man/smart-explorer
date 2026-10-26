package app.smartexplorer.android.task

import android.os.SystemClock
import app.smartexplorer.android.api.ScreenBufferBudget
import app.smartexplorer.android.core.AndroidImageFixtureBridge as Bridge
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.screen.EncoderConfig
import app.smartexplorer.android.screen.ScreenCaptureBudget
import app.smartexplorer.android.screen.ScreenEncoder
import app.smartexplorer.android.screen.ScreenFrameLimits
import app.smartexplorer.android.screen.ScreenSyntheticRetirement
import app.smartexplorer.android.screen.SyntheticSurfaceSource
import app.smartexplorer.android.ui.screen.ScreenPullBuffers
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json

/** Real W/X admission, kept until all observed platform owners have actually ended. */
internal class ScreenCaptureFixture private constructor(val descriptor:kotlinx.serialization.json.JsonObject) : AutoCloseable {
    val session=descriptor.long("session")
    val width=descriptor.int("width")
    val height=descriptor.int("height")
    val viewerBudget:ScreenBufferBudget?=descriptor["viewer_budget"]?.let {
        Core.json.decodeFromString<ScreenBufferBudget>(it.toString())
    }
    private var openedBudget:ScreenCaptureBudget?=null
    fun budget():ScreenCaptureBudget = openedBudget ?: ScreenCaptureBudget.open(session,width,height,descriptor.int("kind")).also {openedBudget=it}
    fun state()=Json.parseToJsonElement(Bridge.captureState(session)).obj()
    fun pulls():ScreenPullBuffers = ScreenPullBuffers({checkNotNull(viewerBudget)},Bridge.captureViewerPullBuffer(session))
    fun releaseReader()=Bridge.captureReleaseDecoderReader(session)
    fun releaseViewer()=Bridge.captureReleaseViewer(session)
    fun consumeStill(generation:Long)=Json.parseToJsonElement(Bridge.captureConsumeStill(session,generation)).obj()
    fun source(encoder:ScreenEncoder,config:EncoderConfig):SyntheticSurfaceSource {
        val lease=encoder.retainSurface()
        return try {SyntheticSurfaceSource(encoder.surface,config.width,config.height,config.fps,lease)}
        catch(failure:Throwable) {lease.close();throw failure}
    }
    fun retireSynthetic(source:SyntheticSurfaceSource?,encoder:ScreenEncoder?) {
        if(source==null) {encoder?.release();return}
        val done=CountDownLatch(1)
        var outcome:Result<Unit>?=null
        ScreenSyntheticRetirement.retireSynthetic(source,checkNotNull(encoder)) { result -> outcome=result;done.countDown() }
        check(done.await(5,TimeUnit.SECONDS)) {"Synthetic worker did not finish; native fixture ownership remains live: ${state()}"}
        checkNotNull(outcome).getOrThrow()
    }
    override fun close() {
        Bridge.captureRetire(session)
        val deadline=SystemClock.uptimeMillis()+5_000
        var observed=state()
        while(observed.objects("generations").any {it.long("live_bytes")!=0L} && SystemClock.uptimeMillis()<deadline) {
            SystemClock.sleep(10);observed=state()
        }
        TaskReport.note("screen.capture.retirement",observed.toString())
        check(observed.objects("generations").all {it.long("live_bytes")==0L}) {"Capture callbacks/resources have not actually ended: $observed"}
        Bridge.captureFinish(session)
    }
    companion object {
        fun open(width:Int,height:Int,kind:Int,decoderReader:Boolean=false):ScreenCaptureFixture {
            runBlocking {Api.awaitReady()}
            val outputWidth=if(kind==ScreenCaptureBudget.ACCESSIBILITY) maxOf(1,width/4) else width
            val outputHeight=if(kind==ScreenCaptureBudget.ACCESSIBILITY) maxOf(1,height/4) else height
            val heap=ScreenFrameLimits.workingBytes()
            val value=Json.parseToJsonElement(Bridge.captureOpen(width,height,outputWidth,outputHeight,kind,heap,decoderReader)).obj()
            require(value.long("session")>0 && value.long("generation")>0 && value.long("heap_offer")==heap)
            require(value.int("width")==width && value.int("height")==height && value.int("kind")==kind)
            TaskReport.note("screen.capture.admission",value.toString())
            return ScreenCaptureFixture(value)
        }
    }
}

/** Cleanup attempts every independent owner; the first failure retains all suppressed evidence. */
internal fun captureCleanup(vararg actions:()->Unit) {
    var failure:Throwable?=null
    actions.forEach { action -> try {action()} catch(error:Throwable) {
        if(failure==null) failure=error else failure!!.addSuppressed(error)
    }}
    failure?.let {throw it}
}
