package app.smartexplorer.android.launcher.apps

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build

/**
 * The system broadcasts that tell a profile was paused, resumed, locked or unlocked (U38-13, U38-14): the work
 * profile's `MANAGED_PROFILE_AVAILABLE`/`UNAVAILABLE` and, from API 35, `PROFILE_AVAILABLE`/`UNAVAILABLE` (also for the
 * private space). They are protected broadcasts, so a receiver registered with [register] in `onStart` and released in
 * [unregister] in `onStop` gets them; every one calls [onChange], which reads the apps again (assumption A35: should a
 * device not send them, `onResume` reads the state anew).
 */
class ProfileReceivers(private val context: Context, private val onChange: () -> Unit) {
    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) = onChange()
    }
    private var registered = false

    fun register() {
        if (registered) return
        val filter = IntentFilter().apply { actions(Build.VERSION.SDK_INT).forEach { addAction(it) } }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            context.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            context.registerReceiver(receiver, filter)
        }
        registered = true
    }

    fun unregister() {
        if (!registered) return
        registered = false
        try {
            context.unregisterReceiver(receiver)
        } catch (_: IllegalArgumentException) {
            // Not registered any more.
        }
    }

    companion object {
        /** The actions to listen for on a device of API [sdk]. */
        fun actions(sdk: Int): List<String> = buildList {
            add(Intent.ACTION_MANAGED_PROFILE_AVAILABLE)
            add(Intent.ACTION_MANAGED_PROFILE_UNAVAILABLE)
            if (sdk >= 35) {
                add(Intent.ACTION_PROFILE_AVAILABLE)
                add(Intent.ACTION_PROFILE_UNAVAILABLE)
            }
        }
    }
}
