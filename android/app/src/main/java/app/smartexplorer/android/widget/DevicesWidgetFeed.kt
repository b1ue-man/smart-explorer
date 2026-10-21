package app.smartexplorer.android.widget

import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import app.smartexplorer.android.R
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.core.VaultChanged
import app.smartexplorer.android.service.LauncherSnapshot
import app.smartexplorer.android.ui.AppNav
import java.io.File
import java.io.IOException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.launch

/**
 * The widget's source in the main process (DSK-M13, U40-16): while the part „Geräte-Widget für fremde Launcher“ is on, it
 * reads the same snapshot as the bridge service ([LauncherSnapshot]), writes `<filesDir>/widget/devices-cache.json` with the
 * rows and the navigation token (the file is app-private) and redraws the widget on every share or vault event, summarising
 * bursts for 250 ms. The widget code itself runs in the launcher process and never starts the core.
 *
 * It runs only when the part is on: [start] by the switch, [startIfEnabled] by `SmartExplorerApp.onCreate` (it returns at
 * once when the provider component is off, so a device that never used the widget starts nothing here), [stop] by the switch.
 */
object DevicesWidgetFeed {
    private const val TAG = "DevicesWidgetFeed"
    private const val DEBOUNCE_MS = 250L
    private const val PROVIDER = "app.smartexplorer.android.widget.DevicesWidgetProvider"

    private val lock = Any()
    private var job: Job? = null

    /** Starts the feed when the provider component is on; does nothing otherwise. */
    fun startIfEnabled(context: Context) {
        val app = context.applicationContext
        val state = app.packageManager.getComponentEnabledSetting(ComponentName(app.packageName, PROVIDER))
        if (state == PackageManager.COMPONENT_ENABLED_STATE_ENABLED) start(app)
    }

    /** Starts the feed (idempotent): one reading now, then one after each burst of events. */
    @OptIn(FlowPreview::class)
    fun start(context: Context) {
        val app = context.applicationContext
        synchronized(lock) {
            if (job?.isActive == true) return
            val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
            job = scope.launch(Dispatchers.Default) {
                launch { refresh(app) }
                Core.events
                    .filter { it is CoreEvent.Share || it is VaultChanged }
                    .debounce(DEBOUNCE_MS)
                    .collect { refresh(app) }
            }
        }
    }

    /** Stops the feed (idempotent); the cache file stays until the switch deletes `<filesDir>/widget/`. */
    fun stop() {
        synchronized(lock) {
            job?.cancel()
            job = null
        }
    }

    /** Deletes `<filesDir>/widget/` (the part went off). */
    fun deleteCache(context: Context) {
        val dir = File(context.applicationContext.filesDir, "widget")
        if (dir.exists() && !dir.deleteRecursively()) Log.w(TAG, "could not delete ${dir.path}")
    }

    /** One reading and one redraw. A failing `share.status` keeps the file as it is. */
    private suspend fun refresh(context: Context) {
        val snapshot = LauncherSnapshot.build(context)
        if (snapshot.error != null) return
        val widget = WidgetSnapshot(WidgetCache.rowsOf(snapshot.data), AppNav.tokenFor(context))
        try {
            WidgetCache.write(context, widget)
        } catch (e: IOException) {
            Log.w(TAG, "devices-cache.json not written", e)
            return
        }
        redraw(context, widget)
    }

    private fun redraw(context: Context, snapshot: WidgetSnapshot) {
        val manager = AppWidgetManager.getInstance(context)
        val ids = manager.getAppWidgetIds(ComponentName(context.packageName, PROVIDER))
        if (ids.isEmpty()) return
        for (id in ids) manager.updateAppWidget(id, DevicesWidgetViews.build(context, id, snapshot, Build.VERSION.SDK_INT))
        // API 30 draws the rows from the list service, which reads the file again.
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) manager.notifyAppWidgetViewDataChanged(ids, R.id.devices_list)
    }
}
