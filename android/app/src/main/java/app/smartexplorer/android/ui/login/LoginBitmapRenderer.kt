package app.smartexplorer.android.ui.login

import android.graphics.Color
import android.graphics.HardwareRenderer
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.RenderNode
import android.graphics.SurfaceTexture
import android.util.Log
import android.view.Surface
import java.util.concurrent.Executor
import java.util.Collections
import java.util.IdentityHashMap

// A failed public teardown must keep its Java Surface/renderer owner reachable
// after View/Compose disposal. Native role admission bounds this map to two
// targets per pane; only confirmed retirement removes an entry.
private val retainedLoginTargets = Collections.synchronizedMap(IdentityHashMap<Any, Any>())

/** Public HWUI owner. All renderer operations and retirements use the pane worker. */
internal class LoginBitmapRenderer(private val pane: LoginImagePane, private val invalidated: () -> Unit = {}) {
    private class Frame(val picture: LoginPicture, val rect: RectF, val presented: () -> Unit) {
        var borrow: LoginDrawBorrow? = null
        var committed = false
        var updated = false
        var submitted = false
        var waiting = true
        var previousTimestamp = 0L
        var consumerCommitPending = false
        fun release() { borrow?.close(); borrow = null; picture.close() }
    }
    private class Target(val texture: SurfaceTexture, val width: Int, val height: Int, val ticket: Long) {
        var surface: Surface? = null
        var renderer: HardwareRenderer? = null
        var node: RenderNode? = null
        @Volatile var frame: Frame? = null
        var pending: Frame? = null
        @Volatile var closed = false
        var ownsTexture = false
        @Volatile var failed = false
        var drainQueued = false
    }
    private val targets = ArrayList<Target>(2)
    private var current: Target? = null
    private val paint = Paint(Paint.FILTER_BITMAP_FLAG)
    private val callbackExecutor = Executor { work -> pane.worker.execute(work) }

    @Synchronized fun attach(texture: SurfaceTexture, width: Int, height: Int): Boolean {
        val old = current
        if (old?.texture === texture && old.width == width && old.height == height && !old.closed) return true
        if (width <= 0 || height <= 0 || pane.handle == 0L) return false
        val ticket = pane.calls.claimTarget(pane.handle, width, height)
        if (ticket == 0L) return false
        val next = try { Target(texture, width, height, ticket) }
        catch (failure: Throwable) { pane.calls.releaseTarget(pane.handle, ticket); throw failure }
        try {
            targets.add(next)
            retainedLoginTargets[next] = this
        } catch (failure: Throwable) {
            targets.remove(next); retainedLoginTargets.remove(next)
            pane.calls.releaseTarget(pane.handle, ticket)
            throw failure
        }
        current = next
        if (old != null) { old.closed = true; pane.worker.execute { retire(old) } }
        pane.worker.execute {
            try {
                if (next.closed) { retire(next); return@execute }
                next.surface = Surface(texture)
                next.renderer = HardwareRenderer()
                next.node = RenderNode("LoginBitmap")
                next.node!!.setPosition(0, 0, width, height)
                next.node!!.setClipToBounds(true)
                next.renderer!!.setSurface(next.surface)
                next.renderer!!.setOpaque(true)
                next.renderer!!.start()
                drain(next)
            } catch (failure: Throwable) { fail(next, failure) }
        }
        return true
    }
    @Synchronized fun render(picture: LoginPicture, rect: RectF, presented: () -> Unit) {
        val target = current ?: return
        if (target.closed || target.failed) return
        val held = picture.retain() ?: return
        val frame = try { Frame(held, RectF(rect), presented) }
        catch (failure: Throwable) { held.close(); throw failure }
        // Exactly one pending picture; StateFlow replacement never retires a recorded borrow.
        // Pending has never recorded a Bitmap borrow, so replacing its reference
        // cannot wait for RenderThread. Coalesce work while a renderer is blocked.
        target.pending?.release()
        target.pending = frame
        if (!target.drainQueued) {
            target.drainQueued = true
            pane.worker.execute {
                synchronized(this) { target.drainQueued = false }
                drain(target)
            }
        }
    }
    @Synchronized fun consumerCommit(texture: SurfaceTexture): Runnable? {
        val target = current?.takeIf { it.texture === texture && !it.closed && !it.failed } ?: return null
        val frame = target.frame ?: return null
        if (frame.consumerCommitPending) return null
        frame.consumerCommitPending = true
        // Capture the frame now, not when the callback later reaches the worker.
        // TextureView's update callback only queues acquisition in AOSP. Read the
        // acquired buffer timestamp after a subsequent window frame commits.
        return Runnable {
            val timestamp = runCatching { texture.timestamp }.getOrNull()
            synchronized(this) { frame.consumerCommitPending = false }
            pane.worker.execute {
                if (timestamp != null && timestamp != frame.previousTimestamp && target.frame === frame) {
                    frame.updated = true
                    complete(target, frame)
                }
            }
        }
    }
    /** true transfers SurfaceTexture release responsibility away from TextureView. */
    @Synchronized fun detach(texture: SurfaceTexture): Boolean {
        val matching = targets.filter { it.texture === texture }
        if (matching.isEmpty()) return false
        matching.last().ownsTexture = true
        matching.forEach { target ->
            target.closed = true
            if (current === target) current = null
            pane.worker.execute { retire(target) }
        }
        return true
    }
    fun clear() { close() }
    @Synchronized fun close() {
        val target = current ?: return
        current = null; target.closed = true
        pane.worker.execute { retire(target) }
    }
    private fun drain(target: Target) {
        if (target.closed || target.failed || target.renderer == null || target.frame?.waiting == true) return
        val frame = synchronized(this) { target.pending.also { target.pending = null } } ?: return
        try {
            clearContent(target)
            if (target.closed) { frame.release(); return }
            frame.previousTimestamp = target.texture.timestamp
            target.frame = frame // includes unsubmitted/partly recorded failure ownership
            val borrow = frame.picture.borrow() ?: run { target.frame = null; frame.release(); return }
            frame.borrow = borrow
            val renderer = target.renderer ?: return
            val node = target.node ?: return
            val canvas = node.beginRecording(target.width, target.height)
            try { canvas.drawColor(Color.BLACK); borrow.draw(canvas, frame.rect, paint) } finally { node.endRecording() }
            renderer.setContentRoot(node)
            renderer.start()
            val result = renderer.createRenderRequest().setVsyncTime(System.nanoTime())
                .setFrameCommitCallback(callbackExecutor) { frame.committed = true; complete(target, frame) }.syncAndDraw()
            frame.submitted = result and (HardwareRenderer.SYNC_FRAME_DROPPED or HardwareRenderer.SYNC_CONTEXT_IS_STOPPED or HardwareRenderer.SYNC_LOST_SURFACE_REWARD_IF_FOUND) == 0
            if (!frame.submitted) { retireFailedFrame(target) } else { complete(target, frame) }
        } catch (failure: Throwable) {
            // A failed old-display-list teardown may precede ownership transfer
            // of this unrecorded candidate. It must not lose its last reference.
            if (target.frame !== frame) frame.release()
            fail(target, failure)
        }
    }
    private fun complete(target: Target, frame: Frame) {
        if (target.closed || target.failed || target.frame !== frame || !frame.waiting || !frame.submitted || !frame.committed || !frame.updated) return
        frame.waiting = false
        frame.presented()
        drain(target)
    }
    private fun clearContent(target: Target) {
        val renderer = target.renderer
        if (renderer != null) { renderer.stop(); renderer.clearContent() }
        target.node?.discardDisplayList()
        target.frame?.release(); target.frame = null
    }
    private fun retireFailedFrame(target: Target) {
        // A dropped/deferred request may still leave a buffer update queued.
        // Retire this producer and require a fresh TextureView/SurfaceTexture;
        // its old update can never acknowledge a different picture.
        target.failed = true
        retire(target)
        invalidated()
    }
    private fun fail(target: Target, failure: Throwable) {
        target.failed = true
        Log.e("LoginBitmap", "Rendererabbau erforderlich", failure)
        retire(target)
        invalidated()
    }
    private fun retire(target: Target) {
        if (synchronized(this) { !targets.contains(target) }) return
        try {
            target.renderer?.stop()
            target.renderer?.clearContent()
            target.node?.discardDisplayList()
            target.renderer?.destroy()
            target.frame?.release(); target.frame = null
            val pending = synchronized(this) { target.pending.also { target.pending = null } }
            pending?.release()
            target.node = null; target.renderer = null
            target.surface?.release(); target.surface = null
            synchronized(this) {
                if (target.ownsTexture) target.texture.release()
                targets.remove(target)
                if (current === target) current = null
            }
            pane.calls.releaseTarget(pane.handle, target.ticket)
            retainedLoginTargets.remove(target)
        } catch (failure: Throwable) {
            // Keep target, recorded bitmap borrow and claim if public teardown did
            // not confirm completion. The native two-target gate prevents growth.
            target.failed = true
            Log.e("LoginBitmap", "Renderbesitz bleibt nach Abbaufehler gebucht", failure)
        }
    }
}
