package app.smartexplorer.android.ui.vaults

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.view.View
import android.view.Window
import android.view.WindowManager
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import java.util.WeakHashMap

/**
 * S6: screens that show cleartext of a vault set `FLAG_SECURE` on the activity window (no
 * screenshots, a blank preview in „Zuletzt verwendet“). A counter per window keeps the flag while
 * any user is composed: a closed sub-screen must not clear the flag of the screen still open.
 * Sheets and dialogs use `securePolicy = SecureFlagPolicy.SecureOn` on their own window instead.
 */
internal object SecureFlags {
    private val users = WeakHashMap<Window, Int>()

    @Synchronized
    fun acquire(window: Window) {
        val count = users[window] ?: 0
        if (count == 0) window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        users[window] = count + 1
    }

    @Synchronized
    fun release(window: Window) {
        val count = (users[window] ?: 1) - 1
        if (count <= 0) {
            users.remove(window)
            window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
        } else {
            users[window] = count
        }
    }

    @Synchronized
    fun held(window: Window): Boolean = (users[window] ?: 0) > 0
}

internal fun Context.findActivity(): Activity? {
    var current: Context? = this
    while (current is ContextWrapper) {
        if (current is Activity) return current
        current = current.baseContext
    }
    return null
}

/** Holds `FLAG_SECURE` on the activity window while composed (S6). */
@Composable
fun SecureWindow() {
    val window = LocalContext.current.findActivity()?.window ?: return
    DisposableEffect(window) {
        SecureFlags.acquire(window)
        onDispose { SecureFlags.release(window) }
    }
}

/**
 * S3: no autofill service sees the password fields of this window while composed: the view tree
 * is excluded (`IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS`), so no password manager offers or
 * stores the vault password. The previous value comes back afterwards.
 */
@Composable
fun NoAutofill() {
    val root = LocalView.current.rootView
    DisposableEffect(root) {
        val before = root.importantForAutofill
        root.importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
        onDispose { root.importantForAutofill = before }
    }
}
