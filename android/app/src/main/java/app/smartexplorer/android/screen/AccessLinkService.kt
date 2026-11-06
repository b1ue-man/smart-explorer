package app.smartexplorer.android.screen

import android.app.Service
import android.content.Intent
import android.os.Binder
import android.os.IBinder
import android.os.Parcel
import android.os.Process
import android.os.RemoteException
import android.util.Log

/**
 * The `:access` end of the link to [ScreenControlService] (not exported, no foreground, no
 * notification): the main process binds it only while it needs gestures, text, global actions,
 * stills or the window list ([AccessLink]). It answers only this app's own uid. Asynchronous
 * results (a gesture's end, a screenshot) go back as one-way calls to the caller's callback.
 */
class AccessLinkService : Service() {
    private val binder = object : Binder() {
        override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
            if (code !in AccessProtocol.FIRST..AccessProtocol.LAST) return super.onTransact(code, data, reply, flags)
            if (Binder.getCallingUid() != Process.myUid()) return false
            val service = ScreenControlService.current()
            when (code) {
                AccessProtocol.PING -> reply?.writeInt(if (service != null) 1 else 0)
                AccessProtocol.GESTURE -> {
                    val callback = data.readStrongBinder()
                    val duration = data.readLong()
                    val count = data.readInt().coerceIn(0, AccessProtocol.MAX_POINTS)
                    val points = List(count) { data.readFloat() to data.readFloat() }
                    if (service == null || points.isEmpty()) answer(callback, false)
                    else service.gesture(GestureMapper.gestureOf(PhoneAction.Stroke(points, duration))) { answer(callback, it) }
                }
                AccessProtocol.TEXT -> {
                    val text = data.readString()
                    val done = service != null && text != null && (if (text == "\n") service.imeEnter() else service.appendText(text))
                    reply?.writeInt(if (done) 1 else 0)
                }
                AccessProtocol.GLOBAL -> {
                    val action = data.readInt()
                    reply?.writeInt(if (service?.performGlobalAction(action) == true) 1 else 0)
                }
                AccessProtocol.WINDOWS -> {
                    val apps = service?.windowApps().orEmpty().take(AccessProtocol.MAX_WINDOWS)
                    reply?.writeInt(apps.size)
                    apps.forEach { (pkg, title) -> reply?.writeString(pkg); reply?.writeString(title) }
                }
                AccessProtocol.STILL -> {
                    val callback = data.readStrongBinder()
                    if (service == null) stillAnswer(callback, null) else service.screenshot { stillAnswer(callback, it) }
                }
            }
            return true
        }
    }

    override fun onBind(intent: Intent?): IBinder = binder

    private fun answer(callback: IBinder?, done: Boolean) {
        callback ?: return
        val parcel = Parcel.obtain()
        try {
            parcel.writeInt(if (done) 1 else 0)
            callback.transact(AccessProtocol.RESULT, parcel, null, IBinder.FLAG_ONEWAY)
        } catch (e: RemoteException) {
            // The main process went away; nobody waits for the result.
        } finally {
            parcel.recycle()
        }
    }

    /**
     * Sends the screenshot's buffer (its file descriptors travel with the call) and colour space, then
     * closes this process's reference; `null` sends a refusal.
     */
    private fun stillAnswer(callback: IBinder?, shot: android.accessibilityservice.AccessibilityService.ScreenshotResult?) {
        val buffer = shot?.hardwareBuffer
        val parcel = Parcel.obtain()
        try {
            if (callback == null) return
            if (shot == null || buffer == null) {
                parcel.writeInt(0)
            } else {
                parcel.writeInt(1)
                parcel.writeInt(shot.colorSpace.id)
                buffer.writeToParcel(parcel, 0)
            }
            callback.transact(AccessProtocol.RESULT, parcel, null, IBinder.FLAG_ONEWAY)
        } catch (e: RemoteException) {
            Log.w(TAG, "Standbild nicht übergeben: ${e.javaClass.simpleName}")
        } catch (e: RuntimeException) {
            Log.w(TAG, "Standbild nicht übergeben: ${e.javaClass.simpleName}")
        } finally {
            parcel.recycle()
            buffer?.close()
        }
    }

    private companion object {
        const val TAG = "SmartExplorerAccess"
    }
}

/** Transactions between [AccessLink] (main process) and [AccessLinkService] (`:access`). */
internal object AccessProtocol {
    const val PING = IBinder.FIRST_CALL_TRANSACTION
    const val GESTURE = IBinder.FIRST_CALL_TRANSACTION + 1
    const val TEXT = IBinder.FIRST_CALL_TRANSACTION + 2
    const val GLOBAL = IBinder.FIRST_CALL_TRANSACTION + 3
    const val WINDOWS = IBinder.FIRST_CALL_TRANSACTION + 4
    const val STILL = IBinder.FIRST_CALL_TRANSACTION + 5
    const val FIRST = PING
    const val LAST = STILL

    /** The answer to a callback (gesture end, still). */
    const val RESULT = IBinder.FIRST_CALL_TRANSACTION

    /**
     * Points of one stroke across the link: 4096 points are 32 KiB, an eighth of the 256 KiB binder
     * buffer Android shares among a process's one-way calls; a gesture is at most
     * [GestureMapper.MAX_STROKE_MS] long, so this covers a point every 2.5 ms.
     */
    const val MAX_POINTS = 4_096

    /** Windows in one answer; [RunningApps] lists at most 32. */
    const val MAX_WINDOWS = 64
}
