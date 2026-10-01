package app.smartexplorer.android.system

import android.net.LinkProperties
import android.net.Network
import app.smartexplorer.android.prefs.AppPrefs
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch

/**
 * Network changes for the Share connection (spec A3, K9): when the default network or its
 * addresses change, the core gets `share.wake {networkChanged:true}` (path re-check plus an
 * immediate probe), debounced by [DEBOUNCE_MS] so a hand-over reported in several steps probes
 * once. Capability updates (signal strength, metering) are no change. The first state after the
 * process start is only the baseline. Fed by the default-network callback of [HostMonitor].
 */
internal object KeepAliveNetwork {
    private const val DEBOUNCE_MS = 2_000L

    /** Keeps the CPU awake through the debounce, so a change while the screen is off still probes. */
    private const val DEBOUNCE_HOLD_MS = DEBOUNCE_MS + 1_000L

    private val started = AtomicBoolean(false)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val states = MutableStateFlow(NetworkState.UNKNOWN)
    private val guard = Any()

    // Guarded by [guard]: the default network and its addresses (`null` until reported).
    private var network: Network? = null
    private var addresses: Set<String>? = null

    fun start() {
        if (!started.compareAndSet(false, true)) return
        scope.launch { observe() }
    }

    fun onAvailable(available: Network) = update {
        if (available != network) {
            network = available
            addresses = null
        }
    }

    fun onLinkProperties(changed: Network, properties: LinkProperties) = update {
        if (changed == network) addresses = properties.linkAddresses.map { it.toString() }.toSet()
    }

    fun onLost(lost: Network) = update {
        if (lost == network) {
            network = null
            addresses = null
        }
    }

    private inline fun update(change: () -> Unit) {
        val next = synchronized(guard) {
            change()
            NetworkState(network, addresses, known = true)
        }
        val current = states.value
        if (current.known && next != current && AppPrefs.shareRunning.value) WakeKeeper.hold(DEBOUNCE_HOLD_MS)
        states.value = next
    }

    private suspend fun observe() {
        var baseline: NetworkState? = null
        // collectLatest cancels the waiting block on every newer state: a debounce without timers
        // of its own, and the blocks never run concurrently.
        states.collectLatest { state ->
            if (!state.known) return@collectLatest
            delay(DEBOUNCE_MS)
            val previous = baseline
            baseline = state
            if (previous == null || previous == state || state.network == null) return@collectLatest
            if (!AppPrefs.shareRunning.value) return@collectLatest
            WakeKeeper.whileHeld(KeepAliveProbe.WAIT_MS) { KeepAliveProbe.run(networkChanged = true) }
        }
    }

    /** A default network (`null` = none) with its addresses; [known] is `false` before the first report. */
    private data class NetworkState(val network: Network?, val addresses: Set<String>?, val known: Boolean) {
        companion object {
            val UNKNOWN = NetworkState(null, null, known = false)
        }
    }
}
