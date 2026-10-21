package app.smartexplorer.android.widget

import android.content.Context
import android.content.Intent
import android.widget.RemoteViews
import android.widget.RemoteViewsService
import app.smartexplorer.android.R

/**
 * The list adapter of the widget on API 30 (DSK-M13, U40-16); from API 31 on the provider hands the rows over directly. It
 * runs in the launcher process and reads `<filesDir>/widget/devices-cache.json` whenever the system asks for new data
 * (`onDataSetChanged`). It binds nothing and starts no process of the main app.
 */
class DevicesWidgetService : RemoteViewsService() {
    override fun onGetViewFactory(intent: Intent): RemoteViewsFactory = Factory(applicationContext)

    private class Factory(private val context: Context) : RemoteViewsFactory {
        private var rows: List<WidgetRow> = emptyList()

        override fun onCreate() = Unit

        override fun onDataSetChanged() {
            rows = WidgetCache.read(context)?.rows.orEmpty()
        }

        override fun onDestroy() {
            rows = emptyList()
        }

        override fun getCount(): Int = rows.size

        override fun getViewAt(position: Int): RemoteViews? = rows.getOrNull(position)?.let { DevicesWidgetViews.rowViews(context, it) }

        override fun getLoadingView(): RemoteViews? = null

        override fun getViewTypeCount(): Int = 1

        override fun getItemId(position: Int): Long = position.toLong()

        override fun hasStableIds(): Boolean = false
    }
}
