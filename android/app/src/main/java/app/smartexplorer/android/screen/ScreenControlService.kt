package app.smartexplorer.android.screen

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.Display
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
import java.util.concurrent.Executors

/**
 * The se accessibility service (plan 2026-10-17, SCR-M8, C1 Android L2, C3): the only way input of
 * another device reaches this phone, and the source of still pictures without the system dialog
 * (at most one per 333 ms). The user switches it on in the Android settings; nothing here turns it
 * on. Input comes only from the core's input channel, which carries the input of the one viewer
 * that holds control with the `ScreenControl` right while a full stream runs. It reads no event
 * content and logs nothing of the screen. [performGlobal] serves the launcher gestures (V6).
 */
class ScreenControlService : AccessibilityService() {
    private val shots = Executors.newSingleThreadExecutor()

    override fun onServiceConnected() {
        super.onServiceConnected()
        instance = this
        ScreenHostControl.accessibilityChanged(applicationContext)
    }

    override fun onUnbind(intent: Intent?): Boolean {
        instance = null
        ScreenHostControl.accessibilityChanged(applicationContext)
        return super.onUnbind(intent)
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        shots.shutdown()
        super.onDestroy()
    }

    // Events are not used: the service exists for gestures, actions, text and stills.
    override fun onAccessibilityEvent(event: AccessibilityEvent?) = Unit

    override fun onInterrupt() = Unit

    /** Dispatches [gesture]; [done] gets whether it completed. */
    fun gesture(gesture: GestureDescription, done: (Boolean) -> Unit) {
        val callback = object : GestureResultCallback() {
            override fun onCompleted(gestureDescription: GestureDescription?) = done(true)

            override fun onCancelled(gestureDescription: GestureDescription?) = done(false)
        }
        if (!dispatchGesture(gesture, callback, null)) done(false)
    }

    /** Appends [text] to the focused input field (`ACTION_SET_TEXT`); `false` without one. */
    fun appendText(text: String): Boolean {
        val node = focusedInput() ?: return false
        val current = if (node.isShowingHintText) "" else node.text?.toString().orEmpty()
        val arguments = Bundle().apply {
            putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, current + text)
        }
        return node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, arguments)
    }

    /** „Enter“ in the focused input field (`ACTION_IME_ENTER`, API 30). */
    fun imeEnter(): Boolean {
        val node = focusedInput() ?: return false
        return node.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)
    }

    /**
     * The input field that has the focus. `findFocus(FOCUS_INPUT)` asks the focused view's node
     * provider, and a Compose view answers it with its own node until its accessibility state
     * caught up with the focus (`AccessibilityInteractionController` then falls back to the view,
     * which takes no text). Then the window is searched for the focused editable node; without one
     * the answer of `findFocus` stays.
     */
    private fun focusedInput(): AccessibilityNodeInfo? {
        val root = rootInActiveWindow ?: return null
        val found = root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
        if (found != null && found.isEditable) return found
        val pending = ArrayDeque<AccessibilityNodeInfo>()
        pending.add(root)
        var visited = 0
        while (pending.isNotEmpty() && visited < MAX_FOCUS_SEARCH_NODES) {
            val node = pending.removeAt(0)
            visited++
            if (node.isFocused && node.isEditable) return node
            for (index in 0 until node.childCount) node.getChild(index)?.let { pending.add(it) }
        }
        return found
    }

    /**
     * One still of the whole display as ARGB_8888 (secure windows show black there, A12), or `null`
     * when Android refused it (more than one per 333 ms, no display).
     */
    fun still(done: (Bitmap?) -> Unit) {
        takeScreenshot(
            Display.DEFAULT_DISPLAY,
            shots,
            object : TakeScreenshotCallback {
                override fun onSuccess(screenshot: ScreenshotResult) {
                    val buffer = screenshot.hardwareBuffer
                    val bitmap = Bitmap.wrapHardwareBuffer(buffer, screenshot.colorSpace)?.copy(Bitmap.Config.ARGB_8888, false)
                    buffer.close()
                    done(bitmap)
                }

                override fun onFailure(errorCode: Int) = done(null)
            },
        )
    }

    /** The apps with a window on screen (package, label), the focused one first. */
    fun windowApps(): List<Pair<String, String>> = windows
        .filter { it.type == AccessibilityWindowInfo.TYPE_APPLICATION }
        .sortedByDescending { it.isFocused }
        .mapNotNull { window -> window.root?.packageName?.toString()?.let { it to (window.title?.toString() ?: it) } }
        .distinctBy { it.first }

    companion object {
        /** Nodes the search for the focused field looks at, at most (a window rarely has more). */
        private const val MAX_FOCUS_SEARCH_NODES = 2_000

        @Volatile
        private var instance: ScreenControlService? = null

        /** The running service, `null` while the user has not switched it on. */
        fun current(): ScreenControlService? = instance

        /** A global action (`GLOBAL_ACTION_*`); `false` when the service does not run (V6). */
        fun performGlobal(action: Int): Boolean = instance?.performGlobalAction(action) ?: false

        /** Whether the user switched the service on in the Android settings. */
        fun enabled(context: Context): Boolean {
            val list = Settings.Secure.getString(context.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES).orEmpty()
            val name = "${context.packageName}/${ScreenControlService::class.java.name}"
            return list.split(':').any { it.equals(name, ignoreCase = true) }
        }

        /**
         * `on`, `off` or `restricted`: a sideloaded app on Android 13+ can only be switched on after
         * „Eingeschränkte Einstellungen zulassen“ in its app info. Android names no such state, so
         * `restricted` means: off, and the app was not installed from a store.
         */
        fun state(context: Context): String = when {
            instance != null || enabled(context) -> "on"
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && sideloaded(context) -> "restricted"
            else -> "off"
        }

        private fun sideloaded(context: Context): Boolean = try {
            val source = context.packageManager.getInstallSourceInfo(context.packageName)
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                source.packageSource != android.content.pm.PackageInstaller.PACKAGE_SOURCE_STORE
        } catch (e: Exception) {
            true
        }
    }
}
