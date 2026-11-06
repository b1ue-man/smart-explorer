package app.smartexplorer.android.screen

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.graphics.Bitmap
import android.graphics.ColorSpace
import android.hardware.HardwareBuffer
import android.os.Binder
import android.os.IBinder
import android.os.Looper
import android.os.Parcel
import android.os.RemoteException
import android.os.SystemClock
import android.util.Log
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The main process's access to [ScreenControlService], which runs in `:access` ([AccessLinkService]).
 * Holders ([acquire]/[release]: screen sessions, the launcher bridge, one window-list reading) keep
 * the binding; it is made on first use, only while the person switched the service on, and dropped
 * with the last holder, so `:access` never keeps the main process alive and the main process binds
 * nothing while nobody needs it. Calls block their (non-main) thread until `:access` answers;
 * results that Android delivers later (a gesture's end, a screenshot) come back through a callback.
 */
internal object AccessLink {
    private const val TAG = "SmartExplorerAccess"

    /**
     * How long a call waits for the binding to connect: Android's own limit for a service to start
     * (`ActiveServices.SERVICE_TIMEOUT`, 20 s); after it the system treats the start as failed.
     */
    private const val CONNECT_WAIT_MS = 20_000L

    private val lock = java.lang.Object()
    private var holders = 0
    private var bound = false
    private var app: Context? = null
    private var remote: IBinder? = null

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, service: IBinder?) {
            synchronized(lock) { remote = service; lock.notifyAll() }
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            // `:access` ended; the binding stays and Android connects again when it restarts.
            synchronized(lock) { remote = null }
        }

        override fun onBindingDied(name: ComponentName?) {
            synchronized(lock) { unbindLocked(); lock.notifyAll() }
        }

        override fun onNullBinding(name: ComponentName?) {
            synchronized(lock) { unbindLocked(); lock.notifyAll() }
        }
    }

    fun acquire(context: Context) {
        synchronized(lock) {
            app = context.applicationContext
            holders++
        }
    }

    fun release() {
        synchronized(lock) {
            holders = (holders - 1).coerceAtLeast(0)
            if (holders == 0) unbindLocked()
        }
    }

    /** Runs [block] with a binding held only for its duration (one reading). */
    fun <T> held(context: Context, block: () -> T): T {
        acquire(context)
        try {
            return block()
        } finally {
            release()
        }
    }

    /** Whether the service runs now (switched on and connected by the system). */
    fun running(): Boolean = call(AccessProtocol.PING) { _, reply -> reply.readInt() == 1 } ?: false

    /** A global action (`GLOBAL_ACTION_*`); `false` when the service does not run or Android refused it. */
    fun global(action: Int): Boolean = call(AccessProtocol.GLOBAL, { it.writeInt(action) }) { _, reply -> reply.readInt() == 1 } ?: false

    /** The apps with a window on screen (package, label), the focused one first; empty without the service. */
    fun windowApps(): List<Pair<String, String>> = call(AccessProtocol.WINDOWS) { _, reply ->
        val count = reply.readInt().coerceIn(0, AccessProtocol.MAX_WINDOWS)
        List(count) { (reply.readString() ?: "") to (reply.readString() ?: "") }.filter { it.first.isNotEmpty() }
    } ?: emptyList()

    /** Runs [action] in the service; [done] gets whether Android carried it out (`false` without the service). */
    fun perform(action: PhoneAction, done: (Boolean) -> Unit) {
        when (action) {
            is PhoneAction.Stroke -> {
                val points = action.points.take(AccessProtocol.MAX_POINTS)
                val callback = callback { data -> done(data.readInt() == 1) }
                val sent = call(AccessProtocol.GESTURE, { data ->
                    data.writeStrongBinder(callback)
                    data.writeLong(action.durationMs)
                    data.writeInt(points.size)
                    points.forEach { (x, y) -> data.writeFloat(x); data.writeFloat(y) }
                }) { _, _ -> true }
                if (sent != true) done(false)
            }
            is PhoneAction.Text -> done(call(AccessProtocol.TEXT, { it.writeString(action.text) }) { _, reply -> reply.readInt() == 1 } ?: false)
            is PhoneAction.Global -> done(global(action.action))
        }
    }

    /**
     * One still of the whole display as ARGB_8888 within [budget], or `null` when Android refused
     * it (more than one per 333 ms, no display, service off). Both budget claims precede the
     * screenshot; a result larger than [expectedWidth] × [expectedHeight] is closed unread.
     */
    fun still(budget: ScreenCaptureBudget, expectedWidth: Int, expectedHeight: Int, done: (ScreenCaptureBitmap?) -> Unit) {
        val expected = ScreenFrameLimits.checkedBytes(expectedWidth, expectedHeight).toLong()
        val hardware = budget.claim(ScreenCaptureBudget.NATIVE_SURFACE, expected)
        val software = try {
            budget.claim(ScreenCaptureBudget.READBACK, expected)
        } catch (error: Throwable) {
            hardware.close(); throw error
        }
        val answered = AtomicBoolean(false)
        val callback = callback { data ->
            if (!answered.compareAndSet(false, true)) return@callback
            var buffer: HardwareBuffer? = null
            var notified = false
            fun deliver(value: ScreenCaptureBitmap?) { notified = true; done(value) }
            var delivered = false
            try {
                if (data.readInt() != 1) { deliver(null); return@callback }
                val colorSpace = colorSpaceOf(data.readInt())
                val created = HardwareBuffer.CREATOR.createFromParcel(data)
                buffer = created
                delivered = picture(created, colorSpace, expected, software, ::deliver)
            } catch (error: RuntimeException) {
                if (!notified) deliver(null)
            } finally {
                buffer?.close(); hardware.close()
                if (!delivered) software.close()
            }
        }
        val sent = try {
            call(AccessProtocol.STILL, { it.writeStrongBinder(callback) }) { _, _ -> true }
        } catch (error: Throwable) {
            software.close(); hardware.close(); throw error
        }
        if (sent != true && answered.compareAndSet(false, true)) {
            software.close(); hardware.close(); done(null)
        }
    }

    /** Copies [buffer] into a software picture charged to [software]; `true` when [deliver] got it. */
    private fun picture(
        buffer: HardwareBuffer,
        colorSpace: ColorSpace,
        expected: Long,
        software: ScreenCaptureLease,
        deliver: (ScreenCaptureBitmap?) -> Unit,
    ): Boolean {
        var wrapper: Bitmap? = null
        var bitmap: Bitmap? = null
        try {
            // A rotated result may use the same pixel count. A
            // larger result is closed before any product copy.
            val actual = ScreenFrameLimits.checkedBytes(buffer.width, buffer.height).toLong()
            if (actual > expected) { deliver(null); return false }
            wrapper = Bitmap.wrapHardwareBuffer(buffer, colorSpace)
            val source = wrapper ?: run { deliver(null); return false }
            if (source.byteCount.toLong() > expected) { deliver(null); return false }
            bitmap = source.copy(Bitmap.Config.ARGB_8888, false)
            val result = bitmap ?: run { deliver(null); return false }
            if (result.allocationByteCount.toLong() > expected) { deliver(null); return false }
            val owned = ScreenCaptureBitmap(result, software)
            bitmap = null
            deliver(owned)
            return true
        } finally {
            bitmap?.recycle()
            // wrapHardwareBuffer owns a distinct hardware reference.
            wrapper?.recycle()
        }
    }

    /** The named colour space of [id] (`ColorSpace.getId`); sRGB for an unnamed one. */
    private fun colorSpaceOf(id: Int): ColorSpace {
        val named = ColorSpace.Named.values()
        return if (id in named.indices) ColorSpace.get(named[id]) else ColorSpace.get(ColorSpace.Named.SRGB)
    }

    /** A binder that takes one asynchronous answer of `:access`. */
    private fun callback(onResult: (Parcel) -> Unit): IBinder = object : Binder() {
        override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
            if (code != AccessProtocol.RESULT) return super.onTransact(code, data, reply, flags)
            onResult(data)
            return true
        }
    }

    /**
     * One synchronous transaction; `null` when no binding is held, the service is off, it did not
     * connect in time, it is called on the main thread (which delivers the connection) or `:access` died.
     */
    private fun <T> call(code: Int, write: (Parcel) -> Unit = {}, read: (Int, Parcel) -> T): T? {
        val binder = binder() ?: return null
        val data = Parcel.obtain()
        val reply = Parcel.obtain()
        return try {
            write(data)
            binder.transact(code, data, reply, 0)
            read(code, reply)
        } catch (e: RemoteException) {
            Log.w(TAG, "Bedienungshilfe nicht erreicht: ${e.javaClass.simpleName}")
            null
        } finally {
            data.recycle()
            reply.recycle()
        }
    }

    private fun binder(): IBinder? {
        synchronized(lock) {
            val context = app ?: return null
            if (holders == 0 || !ScreenControlService.enabled(context)) return null
            if (!bound) bindLocked(context)
            if (Looper.myLooper() == Looper.getMainLooper()) return remote
            val deadline = SystemClock.uptimeMillis() + CONNECT_WAIT_MS
            while (bound && remote == null) {
                val left = deadline - SystemClock.uptimeMillis()
                if (left <= 0) break
                try {
                    lock.wait(left)
                } catch (e: InterruptedException) {
                    Thread.currentThread().interrupt()
                    break
                }
            }
            return remote
        }
    }

    private fun bindLocked(context: Context) {
        bound = try {
            context.bindService(Intent(context, AccessLinkService::class.java), connection, Context.BIND_AUTO_CREATE)
        } catch (e: SecurityException) {
            Log.w(TAG, "Bedienungshilfe nicht gebunden", e)
            false
        }
        if (!bound) unbindQuietly(context)
    }

    private fun unbindLocked() {
        val context = app
        if (bound && context != null) unbindQuietly(context)
        bound = false
        remote = null
    }

    private fun unbindQuietly(context: Context) {
        try {
            context.unbindService(connection)
        } catch (e: IllegalArgumentException) {
            // Not registered: nothing to undo.
        }
    }
}
