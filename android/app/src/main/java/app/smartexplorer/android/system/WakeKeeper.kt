package app.smartexplorer.android.system

import android.content.Context
import android.os.PowerManager
import android.os.SystemClock
import android.util.Log

/**
 * Short partial wake locks for Share work while the screen is off (spec A5, K3): the core asks
 * through the `wake` event ([hold]), keep-alive probes hold their own lock for their duration
 * ([whileHeld]). Android honours them in Doze only for a foreground-service process (or one on
 * the battery allowlist), so they complement the background service, never replace it
 * (android-background-reachability.md §1.1). [init] runs in `SmartExplorerApp.onCreate`; before
 * that every request is ignored.
 */
object WakeKeeper {
    private const val TAG = "SmartExplorerWake"

    // "app:purpose" names, as `dumpsys power` and batterystats show them.
    private const val ACTIVITY_TAG = "SmartExplorer:share-activity"
    private const val PROBE_TAG = "SmartExplorer:share-probe"

    /**
     * Upper bound of one [hold]: twice the longest request of the contract (60 s for incoming
     * streams, renewed every 30 s while they run), so a runaway request cannot drain the battery.
     */
    private const val MAX_HOLD_MS = 120_000L

    @Volatile
    private var power: PowerManager? = null

    private val guard = Any()

    // Guarded by [guard].
    private var activityLock: PowerManager.WakeLock? = null
    private var heldUntil = 0L

    fun init(context: Context) {
        power = context.applicationContext.getSystemService(PowerManager::class.java)
    }

    /**
     * Keeps the CPU awake for [ms] from now (event `wake`). A running hold that lasts longer is
     * kept as it is; a longer request extends it.
     */
    fun hold(ms: Long) {
        val manager = power ?: return
        if (ms <= 0) return
        val duration = ms.coerceAtMost(MAX_HOLD_MS)
        synchronized(guard) {
            val until = SystemClock.elapsedRealtime() + duration
            val lock = activityLock ?: newLock(manager, ACTIVITY_TAG).also { activityLock = it }
            if (lock.isHeld && until <= heldUntil) return
            try {
                // Not reference-counted: acquiring again replaces the pending timeout instead of
                // stacking a second hold.
                lock.acquire(duration)
                heldUntil = until
            } catch (e: RuntimeException) {
                Log.w(TAG, "wake lock not taken", e)
            }
        }
    }

    /**
     * Runs [block] under a wake lock of its own, released when [block] ends and at the latest
     * after [maxMs] (keep-alive probe, network change).
     */
    suspend fun whileHeld(maxMs: Long, block: suspend () -> Unit) {
        val lock = power?.let { newLock(it, PROBE_TAG) }
        try {
            lock?.acquire(maxMs)
        } catch (e: RuntimeException) {
            Log.w(TAG, "probe wake lock not taken", e)
        }
        try {
            block()
        } finally {
            try {
                if (lock?.isHeld == true) lock.release()
            } catch (e: RuntimeException) {
                Log.w(TAG, "probe wake lock not released", e)
            }
        }
    }

    private fun newLock(manager: PowerManager, tag: String): PowerManager.WakeLock =
        manager.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, tag).apply { setReferenceCounted(false) }
}
