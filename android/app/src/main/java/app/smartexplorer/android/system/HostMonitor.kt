package app.smartexplorer.android.system

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.wifi.WifiManager
import android.os.BatteryManager
import android.os.PowerManager
import android.util.Log
import androidx.core.content.ContextCompat
import app.smartexplorer.android.api.HostStateArgs
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.core.CoreException
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable

/**
 * Reports the host state to the core (`sys.hostState`: power saving, metered network, Wi-Fi,
 * charging, UI in the foreground) so the daemon's auto-pause works on real values, and holds a
 * Wi-Fi multicast lock while Share is online (LAN presence/discovery needs multicast).
 * Started once per process by [app.smartexplorer.android.service.BackgroundController].
 */
object HostMonitor {
    private const val TAG = "SmartExplorerHost"
    private const val LOCK_TAG = "smart-explorer-share"

    private val started = AtomicBoolean(false)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val pushRequests = MutableStateFlow(0)
    private val shareOnlineFlow = MutableStateFlow(false)
    private val lockGuard = Any()
    private var multicastLock: WifiManager.MulticastLock? = null

    @Volatile
    private var foreground = false

    /** `true` while the Share service runs (last `share.status`). */
    val shareOnline: StateFlow<Boolean> = shareOnlineFlow.asStateFlow()

    fun start(context: Context) {
        if (!started.compareAndSet(false, true)) return
        val app = context.applicationContext
        val filter = IntentFilter().apply {
            addAction(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED)
            addAction(BatteryManager.ACTION_CHARGING)
            addAction(BatteryManager.ACTION_DISCHARGING)
        }
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) = requestPush()
        }
        ContextCompat.registerReceiver(app, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        registerNetworkCallback(app)
        scope.launch { pushRequests.collect { push(app) } }
        scope.launch { Core.events.collect { if (it is CoreEvent.Share) refreshShare(app) } }
        scope.launch { refreshShare(app) }
    }

    /** UI visibility (from `BackgroundController.onUiVisible`). */
    fun setForeground(visible: Boolean) {
        foreground = visible
        requestPush()
    }

    /** Measures synchronously and sends `sys.hostState` (the catch-up worker calls this first). */
    suspend fun pushNow(context: Context) {
        push(context.applicationContext)
    }

    /** Current host state, measured synchronously. */
    fun measure(context: Context): HostStateArgs {
        val power = context.getSystemService(PowerManager::class.java)
        val battery = context.getSystemService(BatteryManager::class.java)
        val connectivity = context.getSystemService(ConnectivityManager::class.java)
        val caps = try {
            connectivity?.let { it.getNetworkCapabilities(it.activeNetwork) }
        } catch (e: SecurityException) {
            Log.w(TAG, "network state not readable", e)
            null
        }
        return HostStateArgs(
            powerSave = power?.isPowerSaveMode == true,
            // No network counts as unmetered: nothing is transferred then anyway.
            metered = caps != null && !caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED),
            wifi = caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true,
            charging = battery?.isCharging == true,
            foreground = foreground,
        )
    }

    private fun requestPush() {
        pushRequests.update { it + 1 }
    }

    private suspend fun push(context: Context) {
        try {
            SyncApi.hostState(measure(context))
        } catch (e: CoreException) {
            Log.w(TAG, "sys.hostState failed: ${e.kind}: ${e.displayText()}")
        }
    }

    private fun registerNetworkCallback(context: Context) {
        val connectivity = context.getSystemService(ConnectivityManager::class.java) ?: return
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = requestPush()

            override fun onLost(network: Network) = requestPush()

            override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities) = requestPush()
        }
        try {
            connectivity.registerDefaultNetworkCallback(callback)
        } catch (e: RuntimeException) {
            // SecurityException or too many callbacks: the state is then pushed on the other events only.
            Log.w(TAG, "network callback not registered", e)
        }
    }

    private suspend fun refreshShare(context: Context) {
        val online = try {
            Core.request<ShareRunning>("share.status").running
        } catch (e: CoreException) {
            Log.w(TAG, "share.status failed: ${e.kind}: ${e.displayText()}")
            return
        }
        shareOnlineFlow.value = online
        updateMulticastLock(context, online)
    }

    private fun updateMulticastLock(context: Context, online: Boolean) {
        synchronized(lockGuard) {
            val lock = multicastLock ?: run {
                val wifi = context.getSystemService(WifiManager::class.java) ?: return
                wifi.createMulticastLock(LOCK_TAG).apply { setReferenceCounted(false) }.also { multicastLock = it }
            }
            try {
                if (online && !lock.isHeld) lock.acquire()
                if (!online && lock.isHeld) lock.release()
            } catch (e: RuntimeException) {
                Log.w(TAG, "multicast lock not changed", e)
            }
        }
    }

    /** The part of `share.status` this monitor needs. */
    @Serializable
    private data class ShareRunning(val running: Boolean = false)
}
