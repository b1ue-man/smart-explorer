package app.smartexplorer.android.widget

import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.Context
import android.os.Build

/**
 * The provider of the widget „Smart Explorer Geräte“ (DSK-M13, U40-16), in the launcher process. `onUpdate` reads the cache
 * file and draws; it binds no service, starts no service and does not start the main process, so a refresh of the widget
 * never wakes the app's core. The data come from the feed in the main process (`DevicesWidgetFeed`) whenever it runs.
 */
class DevicesWidgetProvider : AppWidgetProvider() {
    override fun onUpdate(context: Context, appWidgetManager: AppWidgetManager, appWidgetIds: IntArray) {
        val snapshot = WidgetCache.read(context)
        for (id in appWidgetIds) {
            appWidgetManager.updateAppWidget(id, DevicesWidgetViews.build(context, id, snapshot, Build.VERSION.SDK_INT))
        }
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
            appWidgetManager.notifyAppWidgetViewDataChanged(appWidgetIds, app.smartexplorer.android.R.id.devices_list)
        }
    }
}
