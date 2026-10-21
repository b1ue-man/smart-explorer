package app.smartexplorer.android.launcher.se

import android.app.ActivityManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.Bundle
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.Message
import android.os.Messenger
import android.os.RemoteException
import android.util.Log
import app.smartexplorer.android.launcher.bridge.BridgeProtocol
import app.smartexplorer.android.launcher.bridge.DevicesData
import app.smartexplorer.android.launcher.bridge.DevicesJson
import app.smartexplorer.android.launcher.bridge.GlobalResult
import app.smartexplorer.android.launcher.bridge.ResultJson
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull

/** The connection of the launcher to the main process (U40-7). */
sealed interface BridgeState {
    /** Not bound: the tiles come from the cache with „nicht aktuell“. */
    data object Idle : BridgeState

    /** Bound or binding, no `DEVICES` yet: tiles show „Verbindet“. */
    data object Binding : BridgeState

    /** A `DEVICES` without error arrived. */
    data object Connected : BridgeState

    /** The bind failed, the main process died, or `share.status` failed ([error]): „Smart Explorer antwortet nicht“ with „Erneut“. */
    data class Unreachable(val error: String? = null) : BridgeState
}

/**
 * The binding of the launcher process to `LauncherBridgeService` in the main process (DSK-M13, U40-7), the rule C3.3:
 * the launcher binds **only** when (a) the main process already runs and the visible page or the dock carries a se
 * element, or (b) the person taps a se tile, „Verbinden“, „Erneut“ or makes a gesture. The activity unbinds in
 * `onStop`. A disconnected service is unbound at once, so the system never restarts the main process for the launcher.
 *
 * The navigation token arrives with the first `DEVICES` and stays in memory; it is never written (U40-7). This class
 * runs in the process `:launcher` and uses no class of the main process.
 */
class SeBridgeClient(private val app: Context, private val onData: (DevicesData) -> Unit) {
    private val _state = MutableStateFlow<BridgeState>(BridgeState.Idle)
    val state: StateFlow<BridgeState> = _state.asStateFlow()

    private val _token = MutableStateFlow<String?>(null)

    /** The token of the installation, from `DEVICES`; kept in memory only. */
    val token: StateFlow<String?> = _token.asStateFlow()

    private val _service = MutableStateFlow<Messenger?>(null)
    private val requests = AtomicInteger(0)
    private val answers = ConcurrentHashMap<Int, CompletableDeferred<GlobalResult>>()
    private var bound = false

    private val inbox = Messenger(
        object : Handler(Looper.getMainLooper()) {
            override fun handleMessage(msg: Message) = onMessage(msg)
        },
    )

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) {
            val service = Messenger(binder)
            _service.value = service
            val subscribe = Message.obtain(null, BridgeProtocol.SUBSCRIBE).apply {
                arg1 = BridgeProtocol.VERSION
                replyTo = inbox
            }
            try {
                service.send(subscribe)
            } catch (e: RemoteException) {
                lost("subscribe failed")
            }
        }

        override fun onServiceDisconnected(name: ComponentName) = lost("service disconnected")

        override fun onBindingDied(name: ComponentName) = lost("binding died")

        override fun onNullBinding(name: ComponentName) = lost("null binding")
    }

    val isBound: Boolean get() = bound

    /** Binds (idempotent). `false` when the system refused; the state is then [BridgeState.Unreachable]. */
    fun bind(): Boolean {
        if (bound) return true
        _state.value = BridgeState.Binding
        val intent = Intent().setComponent(ComponentName(app.packageName, BridgeProtocol.SERVICE_CLASS))
        bound = try {
            app.bindService(intent, connection, Context.BIND_AUTO_CREATE)
        } catch (e: SecurityException) {
            Log.w(TAG, "bindService refused", e)
            false
        }
        if (!bound) {
            // A bind that returns false still needs its unbind.
            runCatching { app.unbindService(connection) }
            _state.value = BridgeState.Unreachable()
        }
        return bound
    }

    /** Unsubscribes and unbinds (idempotent): `onStop` of the activity. */
    fun unbind() {
        val service = _service.value
        if (bound) {
            if (service != null) {
                runCatching {
                    service.send(Message.obtain(null, BridgeProtocol.UNSUBSCRIBE).apply { replyTo = inbox })
                }
            }
            runCatching { app.unbindService(connection) }
        }
        bound = false
        _service.value = null
        answers.values.forEach { it.complete(GlobalResult(false, BridgeProtocol.REASON_UNSUPPORTED)) }
        answers.clear()
        _state.value = BridgeState.Idle
    }

    /** The main process died or the bind failed: unbind so the system does not start it again (Ä22). */
    private fun lost(reason: String) {
        Log.w(TAG, "bridge lost: $reason")
        if (bound) runCatching { app.unbindService(connection) }
        bound = false
        _service.value = null
        answers.values.forEach { it.complete(GlobalResult(false, BridgeProtocol.REASON_UNSUPPORTED)) }
        answers.clear()
        _state.value = BridgeState.Unreachable()
    }

    private fun onMessage(msg: Message) {
        when (msg.what) {
            BridgeProtocol.DEVICES -> {
                val text = msg.data?.getString(BridgeProtocol.KEY_JSON) ?: return
                val data = DevicesJson.parse(text)
                if (data == null) {
                    Log.w(TAG, "DEVICES unreadable")
                    return
                }
                if (data.error != null) {
                    _state.value = BridgeState.Unreachable(data.error)
                    return
                }
                data.navToken?.let { _token.value = it }
                _state.value = BridgeState.Connected
                onData(data.copy(navToken = null))
            }
            BridgeProtocol.RESULT -> {
                val result = ResultJson.parse(msg.data?.getString(BridgeProtocol.KEY_JSON).orEmpty())
                answers.remove(msg.arg2)?.complete(result ?: GlobalResult(false, BridgeProtocol.REASON_UNSUPPORTED))
            }
        }
    }

    /** The token, waiting at most [timeoutMs] for the first `DEVICES` after binding (case (b), U40-7); `null` on timeout. */
    suspend fun awaitToken(timeoutMs: Long): String? {
        _token.value?.let { return it }
        return withTimeoutOrNull(timeoutMs) { _token.filterNotNull().first() }
    }

    /**
     * Sends a global action through the service (gesture, U40-18): binds when needed (case (b): the person's act),
     * waits for the answer at most [timeoutMs]. `null` means no answer in time.
     */
    suspend fun sendGlobalAction(action: String, timeoutMs: Long = ANSWER_TIMEOUT_MS): GlobalResult? {
        if (!bind()) return null
        val request = requests.incrementAndGet()
        val answer = CompletableDeferred<GlobalResult>()
        answers[request] = answer
        val result = withTimeoutOrNull(timeoutMs) {
            val service = _service.filterNotNull().first()
            val message = Message.obtain(null, BridgeProtocol.GLOBAL_ACTION).apply {
                data = Bundle().apply { putString(BridgeProtocol.KEY_ACTION, action) }
                arg2 = request
                replyTo = inbox
            }
            try {
                service.send(message)
            } catch (e: RemoteException) {
                return@withTimeoutOrNull GlobalResult(false, BridgeProtocol.REASON_UNSUPPORTED)
            }
            answer.await()
        }
        answers.remove(request)
        return result
    }

    companion object {
        private const val TAG = "SeBridgeClient"

        /** UX limit for the answer to a gesture (U40-18), not a technical limit. */
        const val ANSWER_TIMEOUT_MS = 2_000L

        /** UX limit for the first `DEVICES` after the person's act (U40-7), not a technical limit. */
        const val FIRST_DATA_TIMEOUT_MS = 3_000L

        /** True when the main process (`processName == packageName`) is among the app's running processes (C3.3). */
        fun isMainProcessRunning(context: Context): Boolean {
            val processes = context.getSystemService(ActivityManager::class.java)?.runningAppProcesses ?: return false
            return processes.any { it.processName == context.packageName }
        }
    }
}
