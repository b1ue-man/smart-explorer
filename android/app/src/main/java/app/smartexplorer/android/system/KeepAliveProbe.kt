package app.smartexplorer.android.system

import android.util.Log
import app.smartexplorer.android.api.ShareApi
import app.smartexplorer.android.api.ShareWake
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.prefs.AppPrefs
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.withTimeoutOrNull

/**
 * One `share.wake` probe of the Share connection (V3) after the wake alarm or a network change.
 * The core answers when its probe ended (at most 12 s); the caller keeps the CPU awake meanwhile.
 */
internal object KeepAliveProbe {
    private const val TAG = "SmartExplorerKeepAlive"

    /**
     * How long a caller waits: the probe's 12 s plus a core that is still starting in a process
     * the alarm just launched. A blocking call that outlasts it keeps running on its own.
     */
    const val WAIT_MS = 20_000L

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /** Probes when Share is set up; `null` when skipped, failed or not finished within [WAIT_MS]. */
    suspend fun run(networkChanged: Boolean): ShareWake? {
        if (!AppPrefs.shareRunning.value) return null
        // Own job: the JNI call cannot be interrupted, so the timeout must not wait for it.
        val call = scope.async { ShareApi.wake(networkChanged) }
        val result = try {
            withTimeoutOrNull(WAIT_MS) { call.await() }
        } catch (e: CoreException) {
            Log.w(TAG, "share.wake failed: ${e.kind}: ${e.message}")
            return null
        }
        if (result == null) {
            Log.w(TAG, "share.wake did not finish within $WAIT_MS ms")
        } else {
            Log.i(TAG, "probe (networkChanged=$networkChanged): ok=${result.ok}, reconnected=${result.reconnected}")
        }
        return result
    }
}
