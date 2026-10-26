package app.smartexplorer.android.ui.screen

import android.graphics.Bitmap
import app.smartexplorer.android.api.ScreenBufferBudget
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.coroutines.coroutineContext

/** One session owns one active CPU job and one conflated next source, including
 * Full/Preview and Surface handovers. Offers, deliveries and disposal run on Main.
 * Old outputs keep their leases until Compose has replaced their draw content. */
internal class ScreenScaleWorker(private val budget:()->ScreenBufferBudget?) {
    class Owner {
        var active=true
        var retainedBytes=0L
    }
    private data class Request(val owner:Owner,val source:Bitmap,val key:List<Int>,
        val identity:ScreenPresentedFrame?,val complete:(ScreenScaled?)->Boolean)
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
    private var pending:Request?=null
    private var runner:Job?=null
    private var activeWork:Deferred<Bitmap>?=null
    private var activeOwner:Owner?=null
    private var installed:ScreenBufferBudget?=null
    private var leased=0L
    @Volatile private var closed=false

    fun owner()=Owner()
    fun submit(owner:Owner,source:Bitmap,key:List<Int>,identity:ScreenPresentedFrame?,complete:(ScreenScaled?)->Boolean) {
        if(closed || !owner.active) return
        pending=Request(owner,source,key,identity,complete)
        if(runner!=null) return
        val job=scope.launch(start=CoroutineStart.LAZY) {
            try {
                while(true) {
                    val request=pending ?: break
                    pending=null
                    if(request.owner.active) process(request)
                }
            } finally {runner=null}
        }
        runner=job;job.start()
    }
    fun detach(owner:Owner) {
        owner.active=false
        if(pending?.owner===owner) pending=null
        if(activeOwner===owner) activeWork?.cancel()
    }
    fun close() {
        closed=true;pending=null;scope.cancel()
    }
    @Synchronized private fun reserve(actual:ScreenBufferBudget,bytes:Long,futureReplacement:Long):Allocation {
        require(installed==null || installed==actual)
        require(bytes>=0 && futureReplacement>=0 && leased<=actual.scalingBytes()-bytes-futureReplacement) {"screen scaling exceeds negotiated working memory"}
        installed=actual;leased+=bytes
        return Allocation(this,bytes)
    }
    @Synchronized private fun release(bytes:Long) {leased-=bytes}

    internal class Allocation(private val worker:ScreenScaleWorker,private var bytes:Long) {
        private var owner:Owner?=null
        fun retain(next:Long,heldBy:Owner) {
            require(next in 0..bytes)
            worker.release(bytes-next);bytes=next;owner=heldBy;heldBy.retainedBytes+=next
        }
        fun close() {owner?.let {it.retainedBytes-=bytes};owner=null;worker.release(bytes);bytes=0}
    }

    private suspend fun process(request:Request) {
        var allocation:Allocation?=null
        var work:Deferred<Bitmap>?=null
        try {
            val actual=budget() ?: throw IllegalArgumentException("screen buffer budget is not accepted")
            val k=request.key
            val plan=ScreenResample.plan(request.source,k[0],k[1],k[2],k[3],k[4],k[5],actual)
            // A viewport which fits once must also finance its next frame while
            // its current output is still visible. Existing output leases count
            // already; only a larger/new target needs this additional admission.
            val future=(plan.retainedBytes-request.owner.retainedBytes).coerceAtLeast(0)
            val owned=reserve(actual,plan.workingBytes,future)
            allocation=owned
            val task=scope.async(Dispatchers.Default,start=CoroutineStart.LAZY) {
                val active=coroutineContext
                ScreenResample.resize(request.source,plan) {active.ensureActive()}
            }
            work=task;activeWork=task;activeOwner=request.owner;task.start()
            val image=task.await()
            if(!request.owner.active || closed) return
            owned.retain(plan.retainedBytes,request.owner)
            val result=ScreenScaled(k,image,request.identity,owned)
            if(request.complete(result)) {
                allocation=null // The composable owns the bitmap and its lease now.
                // At most one output waits for Compose, not one per decoded frame.
                result.applied.await()
            }
        } catch(cancelled:CancellationException) {
            coroutineContext.ensureActive()
        } catch(_:OutOfMemoryError) {
            if(request.owner.active && !closed) request.complete(null)
        } catch(_:RuntimeException) {
            if(request.owner.active && !closed) request.complete(null)
        } finally {
            // A cancelled CPU job still owns its arrays until it has stopped.
            withContext(NonCancellable) {work?.cancelAndJoin()}
            activeWork=null;activeOwner=null
            allocation?.close()
        }
    }
}

internal class ScreenScaled(val key:List<Int>,val bitmap:Bitmap,val identity:ScreenPresentedFrame?,
    private val allocation:ScreenScaleWorker.Allocation) {
    val applied=CompletableDeferred<Unit>()
    fun close() {allocation.close();applied.complete(Unit)}
}
