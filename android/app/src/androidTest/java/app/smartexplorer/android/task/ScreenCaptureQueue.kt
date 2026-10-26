package app.smartexplorer.android.task

import android.os.SystemClock
import app.smartexplorer.android.screen.ScreenCaptureBudget
import app.smartexplorer.android.screen.ScreenCaptureBuffer
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.ArrayDeque

/** Two real AU owners across producer, queue, held -3 retry and active pull. */
internal class ScreenCaptureQueue(private val budget:ScreenCaptureBudget,private val fullGop:Long,frameBase:Long) : AutoCloseable {
    private class Unit(val bytes:ScreenCaptureBuffer,val pts:Long,val key:Boolean)
    private val lock=java.lang.Object()
    private val queued=ArrayDeque<Unit>()
    private var held:Unit?=null
    private var slots=0
    private var producers=0
    private var closed=false
    private var started=false
    private var deliveredFrame=frameBase
    @Volatile var failure:Throwable?=null
        private set
    fun push(data:ByteBuffer,offset:Int,length:Int,pts:Long,key:Boolean):Int {
        synchronized(lock) {
            while(!closed && slots==2) lock.wait(100)
            if(closed) return -1
            slots++;producers++
        }
        var bytes:ScreenCaptureBuffer?=null
        var transferred=false
        try {
            bytes=budget.tryBuffer(ScreenCaptureBudget.TRANSPORT,length) ?: error("Native AU transport claim refused")
            bytes.access {target -> target.put(data.duplicate().apply {clear();position(offset);limit(Math.addExact(offset,length))})}
            synchronized(lock) {
                if(!closed) {queued.addLast(Unit(checkNotNull(bytes),pts,key));transferred=true;lock.notifyAll();return 0}
            }
            return -1
        } catch(error:Throwable) {
            failure=error
            synchronized(lock) {closed=true;lock.notifyAll()}
            return -1
        } finally {
            if(!transferred) bytes?.close()
            synchronized(lock) {producers--;if(!transferred) slots--;lock.notifyAll()}
        }
    }
    fun pull(dst:ByteBuffer,timeoutMs:Int):Int = synchronized(lock) {
        val deadline=SystemClock.uptimeMillis()+timeoutMs
        while(!closed && held==null && queued.isEmpty()) {
            val left=deadline-SystemClock.uptimeMillis()
            if(left<=0) return@synchronized -1
            lock.wait(left)
        }
        if(closed) return@synchronized -2
        val next=held ?: queued.pollFirst() ?: return@synchronized -1
        held=next
        if(!started && !next.key) {releaseHeld();return@synchronized -1}
        val length=next.bytes.capacity
        if(dst.capacity()<32+length) {
            dst.order(ByteOrder.LITTLE_ENDIAN).putInt(12,16+length)
            return@synchronized -3
        }
        try {
            val flags=(if(next.key)1 else 0) or (if(!started)4 else 0) or (1 shl 16) or 0x100 or 0x80
            dst.order(ByteOrder.LITTLE_ENDIAN)
            dst.putLong(0,next.pts);dst.putInt(8,flags);dst.putInt(12,16+length)
            dst.putLong(16,fullGop);dst.putLong(24,++deliveredFrame);dst.position(32)
            next.bytes.access {dst.put(it)}
            started=true
            32+length
        } finally {releaseHeld()}
    }
    private fun releaseHeld() {held?.bytes?.close();held=null;slots--;lock.notifyAll()}
    override fun close() = synchronized(lock) {
        closed=true
        if(held!=null) releaseHeld()
        while(queued.isNotEmpty()) {queued.removeFirst().bytes.close();slots--}
        lock.notifyAll()
        val deadline=SystemClock.uptimeMillis()+5_000
        while(producers!=0 && SystemClock.uptimeMillis()<deadline) lock.wait(50)
        check(producers==0 && slots==0) {"AU producer has not relinquished its real owner"}
    }
    fun checkHealthy() {failure?.let {throw AssertionError("AU transfer failed",it)}}
}
