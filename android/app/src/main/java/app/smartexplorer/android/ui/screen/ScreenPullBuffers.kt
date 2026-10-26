package app.smartexplorer.android.ui.screen

import app.smartexplorer.android.api.ScreenBufferBudget
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** A v2 session allocates its sole DirectBuffer once, after Accepted binds W.
 * Growing direct buffers would leave the old native allocation waiting for GC. */
class ScreenPullBuffers internal constructor(private val offered:()->ScreenBufferBudget?) {
    var buffer:ByteBuffer=ByteBuffer.allocateDirect(32).order(ByteOrder.LITTLE_ENDIAN)
        private set
    private var installed:ScreenBufferBudget?=null
    val budget:ScreenBufferBudget get()=installed ?: error("Missing accepted buffer budget")
    /** Called only after the preceding decoder thread has relinquished all IO. */
    fun takeFrom(previous:ScreenPullBuffers?) {
        if(previous==null || previous===this || previous.installed==null) return
        buffer=previous.buffer;installed=previous.installed
        previous.buffer=ByteBuffer.allocateDirect(32);previous.installed=null
    }
    fun prepare():Boolean {
        val next=offered() ?: return false
        if(installed==null) {
            buffer=ByteBuffer.allocateDirect(if(next.legacy) minOf(1 shl 20,next.pullBytes) else next.pullBytes)
                .order(ByteOrder.LITTLE_ENDIAN)
            installed=next
        } else require(installed==next) {"Buffer contract changed within the session"}
        return true
    }
    fun forPayload(length:Int):ByteBuffer? {
        if(length<=0 || length.toLong()+16>budget.pullBytes) return null
        val total=length+16
        if(buffer.capacity()<total) {
            if(!budget.legacy) return null
            // The unversioned path retains its previous bounded growth behavior.
            buffer=ByteBuffer.allocateDirect(total).order(ByteOrder.LITTLE_ENDIAN)
        }
        return buffer
    }
    fun acceptsPayload(length:Int,tiles:Boolean):Boolean =
        length>=0 && length<=if(tiles) budget.imageBytes else budget.recordBytes
}
