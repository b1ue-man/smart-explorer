package app.smartexplorer.android.service

import android.accessibilityservice.AccessibilityService
import android.app.Service
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.Message
import android.os.Messenger
import android.os.Process
import android.os.RemoteException
import android.os.TransactionTooLargeException
import android.util.Log
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.core.VaultChanged
import app.smartexplorer.android.launcher.bridge.BridgeProtocol
import app.smartexplorer.android.launcher.bridge.DevicesJson
import app.smartexplorer.android.launcher.bridge.GlobalResult
import app.smartexplorer.android.launcher.bridge.ResultJson
import app.smartexplorer.android.screen.ScreenControlService
import app.smartexplorer.android.ui.AppNav
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * The main process's end of the launcher bridge (DSK-M13, U40-6): a bound service the launcher process `:launcher`
 * talks to over a `Messenger`. It reads the core on behalf of the launcher (one reading per snapshot, see
 * [LauncherSnapshot]), hands the navigation token over and carries out the two global actions of the gestures.
 *
 * It is not exported (only this app's processes can bind it), it drops every message whose `sendingUid` is not the
 * app's own, it starts no foreground service, shows no notification and holds no wake lock. It lives only while a
 * client is bound (`BIND_AUTO_CREATE`, no `startService`); the launcher binds it only under the rule C3.3
 * (`SeBridgeClient`), so it never starts the main process by itself.
 */
class LauncherBridgeService : Service() {
    private val thread = HandlerThread("launcher-bridge")
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val clients = CopyOnWriteArrayList<Messenger>()
    private val pending = AtomicReference<Job?>(null)
    private lateinit var messenger: Messenger

    override fun onCreate() {
        super.onCreate()
        thread.start()
        messenger = Messenger(
            object : Handler(thread.looper) {
                override fun handleMessage(msg: Message) = onMessage(msg)
            },
        )
        scope.launch {
            Core.events.collect { event ->
                if (event is CoreEvent.Share || event is VaultChanged) rebuildSoon()
            }
        }
    }

    override fun onBind(intent: Intent?): IBinder = messenger.binder

    override fun onDestroy() {
        scope.cancel()
        clients.clear()
        thread.quitSafely()
        super.onDestroy()
    }

    private fun onMessage(msg: Message) {
        // Only the app's own processes may talk to the bridge (C3.3).
        if (msg.sendingUid != Process.myUid()) {
            Log.w(TAG, "message ${msg.what} from uid ${msg.sendingUid} dropped")
            return
        }
        when (msg.what) {
            BridgeProtocol.SUBSCRIBE -> subscribe(msg)
            BridgeProtocol.UNSUBSCRIBE -> msg.replyTo?.let { clients.remove(it) }
            BridgeProtocol.GLOBAL_ACTION -> globalAction(msg)
        }
    }

    private fun subscribe(msg: Message) {
        val client = msg.replyTo ?: return
        if (msg.arg1 != BridgeProtocol.VERSION) {
            deliver(client, DevicesJson.encodeError(BridgeProtocol.ERROR_VERSION))
            return
        }
        if (clients.none { it == client }) clients.add(client)
        scope.launch(Dispatchers.IO) { deliver(client, devicesJson()) }
    }

    /** Statuses come in bursts and one reading costs three bridge calls: summarise for [DEBOUNCE_MS] (U40-6). */
    private fun rebuildSoon() {
        if (clients.isEmpty()) return
        val job = scope.launch(Dispatchers.IO) {
            delay(DEBOUNCE_MS)
            if (clients.isEmpty()) return@launch
            val json = devicesJson()
            clients.forEach { deliver(it, json) }
        }
        pending.getAndSet(job)?.cancel()
    }

    private suspend fun devicesJson(): String {
        val snapshot = LauncherSnapshot.build(this)
        return snapshot.error?.let { DevicesJson.encodeError(it) } ?: LauncherSnapshot.toJson(snapshot, AppNav.tokenFor(this))
    }

    /** Sends `DEVICES`; a message the Binder cannot carry becomes the short error message (A50). */
    private fun deliver(client: Messenger, json: String) {
        try {
            client.send(message(BridgeProtocol.DEVICES, json))
        } catch (e: TransactionTooLargeException) {
            Log.w(TAG, "DEVICES too large (${json.length} characters)", e)
            try {
                client.send(message(BridgeProtocol.DEVICES, DevicesJson.encodeError(BridgeProtocol.ERROR_TOO_LARGE)))
            } catch (_: RemoteException) {
                clients.remove(client)
            }
        } catch (_: RemoteException) {
            clients.remove(client)
        }
    }

    private fun globalAction(msg: Message) {
        val reply = msg.replyTo ?: return
        val result = performAction(msg.data?.getString(BridgeProtocol.KEY_ACTION))
        try {
            reply.send(message(BridgeProtocol.RESULT, ResultJson.encode(result)).apply { arg2 = msg.arg2 })
        } catch (_: RemoteException) {
            clients.remove(reply)
        }
    }

    /** U40-6: unknown action → `unsupported`; service off → `accessibility_off`; the service refuses → `unsupported`. */
    private fun performAction(action: String?): GlobalResult {
        val code = when (action) {
            BridgeProtocol.ACTION_NOTIFICATIONS -> AccessibilityService.GLOBAL_ACTION_NOTIFICATIONS
            BridgeProtocol.ACTION_LOCK_SCREEN -> AccessibilityService.GLOBAL_ACTION_LOCK_SCREEN
            else -> return GlobalResult(false, BridgeProtocol.REASON_UNSUPPORTED)
        }
        if (!ScreenControlService.enabled(this)) return GlobalResult(false, BridgeProtocol.REASON_ACCESSIBILITY_OFF)
        return if (ScreenControlService.performGlobal(code)) GlobalResult(true) else GlobalResult(false, BridgeProtocol.REASON_UNSUPPORTED)
    }

    private fun message(what: Int, json: String): Message = Message.obtain(null, what).apply {
        data = Bundle().apply { putString(BridgeProtocol.KEY_JSON, json) }
    }

    private companion object {
        const val TAG = "LauncherBridge"

        /** Time to summarise status bursts (U40-6). */
        const val DEBOUNCE_MS = 250L
    }
}
