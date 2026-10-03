package app.smartexplorer.android.system

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.app.AppOpsManager
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.wifi.WifiManager
import android.os.BatteryManager
import android.os.PowerManager
import android.os.SystemClock
import android.util.Log
import androidx.core.content.ContextCompat
import app.smartexplorer.android.api.HostStateArgs
import app.smartexplorer.android.api.ShareApi
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncApi.displayText
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.service.BackgroundController
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** What the background notification shows of the Share state; equal snapshots do not re-render. */
data class ShareSnapshot(
    val online: Boolean = false,
    val connected: Boolean = false,
    /** No Share server configured: reachable in the local network only. */
    val lanOnly: Boolean = false,
    val idleSupported: Boolean? = null,
)

/**
 * Host side of the core: reports the host state (`sys.hostState`: power saving, metered network,
 * Wi-Fi, charging, UI visible, scheduling deferred) so the daemon's auto-pause, the Share idle mode
 * and the scheduling gate work on real values; follows `share.status` for the background service
 * ([share], the persisted "Share eingerichtet" flag) and holds the Wi-Fi multicast lock only where
 * mDNS is needed (spec A3). Forwards default-network changes to [KeepAliveNetwork].
 * Started once per process by [BackgroundController].
 */
object HostMonitor {
    private const val TAG = "SmartExplorerHost"
    private const val LOCK_TAG = "smart-explorer-share"

    /**
     * How long Share must stay off before "Share eingerichtet" is cleared and the background
     * service stops: the poller reports every 60 s in the background, so the empty snapshot
     * before its first poll or a worker restart never ends the service.
     */
    private const val SETTLE_MS = 120_000L

    private val started = AtomicBoolean(false)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val pushRequests = MutableStateFlow(0)
    private val shareRefreshes = MutableStateFlow(0)
    private val pushLock = Mutex()
    private val shareFlow = MutableStateFlow(ShareSnapshot())
    private val hostStateFlow = MutableStateFlow<HostStateArgs?>(null)
    private val workerRuns = AtomicInteger(0)
    private val lockGuard = Any()
    private var multicastLock: WifiManager.MulticastLock? = null

    @Volatile
    private var appContext: Context? = null

    @Volatile
    private var foreground = false
    @Volatile private var serviceAvailable = false

    @Volatile
    private var lastStatus: ShareStatus? = null

    // Guarded by [pushLock].
    private var lastPushed: HostStateArgs? = null

    // Touched only by the single share-refresh collector.
    private var offlineSince: Long? = null
    private var settleCheck: Job? = null

    /** Share state of the latest `share.status` (follows the core's `share` events). */
    val share: StateFlow<ShareSnapshot> = shareFlow.asStateFlow()

    /** Host state last sent to the core (`null` before the first); auto-pause follows it. */
    val hostState: StateFlow<HostStateArgs?> = hostStateFlow.asStateFlow()

    fun start(context: Context) {
        if (!started.compareAndSet(false, true)) return
        val app = context.applicationContext
        appContext = app
        val filter = IntentFilter().apply {
            addAction(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED)
            addAction(BatteryManager.ACTION_CHARGING)
            addAction(BatteryManager.ACTION_DISCHARGING)
            addAction(Intent.ACTION_SCREEN_ON)
            addAction(Intent.ACTION_SCREEN_OFF)
        }
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) = requestPush()
        }
        ContextCompat.registerReceiver(app, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        KeepAliveNetwork.start()
        registerNetworkCallback(app)
        MediaStoreSync.start(app)
        try {
            AppOpsManager.permissionToOp(android.Manifest.permission.MANAGE_EXTERNAL_STORAGE)?.let { operation ->
                app.getSystemService(AppOpsManager::class.java)?.startWatchingMode(
                    operation, app.packageName,
                    AppOpsManager.OnOpChangedListener { _, _ -> requestPush() },
                )
            }
        } catch (e: RuntimeException) { Log.w(TAG, "Dateizugriffsänderungen werden periodisch geprüft", e) }
        // Each StateFlow collector runs one request at a time and skips superseded ones.
        scope.launch { pushRequests.collect { push(app, force = false) } }
        // `deferScheduling` follows the mode; the first value also sends the first host state,
        // which replaces the daemon's start default (scheduling deferred until then, K1).
        scope.launch { AppPrefs.bgMode.collect { requestPush() } }
        scope.launch { shareRefreshes.collect { refreshShare(app) } }
        scope.launch { Core.events.collect { event -> when (event) {
            is CoreEvent.Share -> requestShareRefresh()
            is CoreEvent.Jobs -> BackgroundController.refreshSchedule(app)
            is CoreEvent.Volumes -> { MediaStoreSync.probe(app, signal = true); AndroidHostFigures.refresh(app, force = true) }
            is CoreEvent.SyncProblem -> Notifications.syncProblem(app, event)
            else -> Unit
        } } }
        scope.launch { while (true) { delay(5_000); push(app, force = false) } }
        scope.launch { while (true) {
            MediaStoreSync.probe(app)
            BackgroundController.refreshSchedule(app)
            AndroidHostFigures.refresh(app)
            delay(60_000)
        } }
    }

    /** UI visibility (from `BackgroundController.onUiVisible`). */
    fun setForeground(visible: Boolean) {
        foreground = visible
        requestPush()
        appContext?.let { updateMulticastLock(it) }
        if (visible) appContext?.let { app -> scope.launch {
            MediaStoreSync.probe(app)
            AndroidHostFigures.refresh(app, force = true)
            BackgroundController.refreshSchedule(app)
        } }
    }

    fun setServiceAvailable(available: Boolean) { serviceAvailable = available; requestPush() }

    /**
     * The worker owns its admitted catch-up jobs. During its bounded window,
     * independent new daemon scheduling is deferred.
     * Sends the measured host state at once; every call needs a matching [endWorkerRun].
     */
    suspend fun beginWorkerRun(context: Context) {
        workerRuns.incrementAndGet()
        push(context.applicationContext, force = true)
        MediaStoreSync.probe(context.applicationContext)
    }

    /** The worker run ended: scheduling is deferred again unless the UI is visible or "Dauerbetrieb" runs. */
    suspend fun endWorkerRun(context: Context) {
        workerRuns.updateAndGet { (it - 1).coerceAtLeast(0) }
        push(context.applicationContext, force = true)
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
        val visible = foreground
        return HostStateArgs(
            powerSave = power?.isPowerSaveMode == true,
            // No network counts as unmetered: nothing is transferred then anyway.
            metered = caps != null && !caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED),
            wifi = caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true,
            charging = battery?.isCharging == true,
            foreground = visible,
            // Only "Dauerbetrieb" runs scheduled jobs in the background; otherwise a process the
            // background service keeps alive leaves them to the periodic worker's run (spec A6).
            deferScheduling = workerRuns.get() > 0 || (!visible &&
                (AppPrefs.bgMode.value != BackgroundController.MODE_PERSISTENT || !serviceAvailable)),
            storageAccess = Permissions.hasAllFilesAccess(),
        )
    }

    private fun requestPush() {
        pushRequests.update { it + 1 }
    }

    private fun requestShareRefresh() {
        shareRefreshes.update { it + 1 }
    }

    /** Sends the host state unless it equals the last one sent ([force]: always). */
    private suspend fun push(context: Context, force: Boolean) {
        pushLock.withLock {
            val state = measure(context)
            if (!force && state == lastPushed) return
            try {
                SyncApi.hostState(state)
                lastPushed = state
                hostStateFlow.value = state
                BackgroundController.refreshSchedule(context)
            } catch (e: CoreException) {
                Log.w(TAG, "sys.hostState failed: ${e.kind}: ${e.displayText()}")
            }
        }
    }

    private fun registerNetworkCallback(context: Context) {
        val connectivity = context.getSystemService(ConnectivityManager::class.java) ?: return
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                KeepAliveNetwork.onAvailable(network)
                requestPush()
            }

            override fun onLost(network: Network) {
                KeepAliveNetwork.onLost(network)
                requestPush()
            }

            override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities) = requestPush()

            override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) =
                KeepAliveNetwork.onLinkProperties(network, linkProperties)
        }
        try {
            connectivity.registerDefaultNetworkCallback(callback)
        } catch (e: RuntimeException) {
            // SecurityException or too many callbacks: the state is then pushed on the other events only.
            Log.w(TAG, "network callback not registered", e)
        }
    }

    private suspend fun refreshShare(context: Context) {
        val status = try {
            ShareApi.status()
        } catch (e: CoreException) {
            Log.w(TAG, "share.status failed: ${e.kind}: ${e.displayText()}")
            return
        }
        lastStatus = status
        shareFlow.value = ShareSnapshot(
            online = status.running,
            connected = status.connected,
            lanOnly = status.server == null,
            idleSupported = status.power.idleSupported,
        )
        updateMulticastLock(context)
        rememberRunning(status.running)
        if (status.running) AndroidHostFigures.refresh(context)
    }

    /** Persists "Share eingerichtet": at once when on, after [SETTLE_MS] of continuous "off". */
    private fun rememberRunning(running: Boolean) {
        if (running) {
            offlineSince = null
            settleCheck?.cancel()
            settleCheck = null
            if (!AppPrefs.shareRunning.value) AppPrefs.setShareRunning(true)
            return
        }
        if (!AppPrefs.shareRunning.value) return
        val now = SystemClock.elapsedRealtime()
        val since = offlineSince ?: now.also { offlineSince = it }
        val remaining = SETTLE_MS - (now - since)
        if (remaining <= 0) {
            offlineSince = null
            AppPrefs.setShareRunning(false)
            return
        }
        if (settleCheck?.isActive != true) {
            settleCheck = scope.launch {
                delay(remaining)
                requestShareRefresh()
            }
        }
    }

    /**
     * Multicast only where mDNS matters (spec A3): every multicast packet in the Wi-Fi wakes the
     * phone while the lock is held. Held while the UI is visible, while a pairing offer or
     * exchange runs, and without a Share server (LAN only: mDNS is the only way to be found).
     */
    private fun multicastWanted(status: ShareStatus?): Boolean {
        if (status == null || !status.running) return false
        if (foreground || status.server == null) return true
        val offer = status.discovery.offer
        val offering = offer != null && (offer.untilMs <= 0 || offer.untilMs > System.currentTimeMillis())
        return offering || status.discovery.exchange?.state == "running"
    }

    private fun updateMulticastLock(context: Context) {
        synchronized(lockGuard) {
            val wanted = multicastWanted(lastStatus)
            val lock = multicastLock ?: run {
                if (!wanted) return
                val wifi = context.getSystemService(WifiManager::class.java) ?: return
                wifi.createMulticastLock(LOCK_TAG).apply { setReferenceCounted(false) }.also { multicastLock = it }
            }
            try {
                if (wanted && !lock.isHeld) lock.acquire()
                if (!wanted && lock.isHeld) lock.release()
            } catch (e: RuntimeException) {
                Log.w(TAG, "multicast lock not changed", e)
            }
        }
    }
}
