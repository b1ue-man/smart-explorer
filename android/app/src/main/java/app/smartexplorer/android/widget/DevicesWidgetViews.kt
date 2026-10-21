package app.smartexplorer.android.widget

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.view.View
import android.widget.RemoteViews
import app.smartexplorer.android.R
import app.smartexplorer.android.launcher.bridge.BridgeProtocol

/**
 * The views of the widget „Smart Explorer Geräte“ (DSK-M13, U40-16): a list of the devices with a dot for online and
 * offline, the state text and a tap that opens the device in the app. Used by the provider in the launcher process and
 * by the feed in the main process. Imports nothing from the main process (U35-11); the activity is named by string.
 *
 * API 31 and later hand the rows over directly (`RemoteCollectionItems`); API 30 binds `DevicesWidgetService`, which
 * reads the cache file on `onDataSetChanged`.
 */
object DevicesWidgetViews {
    const val TEXT_EMPTY = "Keine Geräte. Öffne Smart Explorer, um Geräte zu verbinden."
    const val TEXT_NO_DATA = "Noch keine Daten."
    private const val MAIN_ACTIVITY = "app.smartexplorer.android.MainActivity"

    /** [snapshot] `null` means the cache file is missing. */
    fun build(context: Context, appWidgetId: Int, snapshot: WidgetSnapshot?, sdk: Int): RemoteViews {
        val views = RemoteViews(context.packageName, R.layout.devices_widget)
        val rows = snapshot?.rows.orEmpty()
        if (rows.isEmpty()) {
            views.setViewVisibility(R.id.devices_list, View.GONE)
            views.setViewVisibility(R.id.devices_empty, View.VISIBLE)
            views.setTextViewText(R.id.devices_empty, if (snapshot == null) TEXT_NO_DATA else TEXT_EMPTY)
            views.setOnClickPendingIntent(R.id.devices_empty, launchApp(context))
            return views
        }
        views.setViewVisibility(R.id.devices_empty, View.GONE)
        views.setViewVisibility(R.id.devices_list, View.VISIBLE)
        if (sdk >= Build.VERSION_CODES.S) {
            val items = RemoteViews.RemoteCollectionItems.Builder().setHasStableIds(false).setViewTypeCount(1)
            rows.forEachIndexed { index, row -> items.addItem(index.toLong(), rowViews(context, row)) }
            views.setRemoteAdapter(R.id.devices_list, items.build())
        } else {
            val service = Intent(context, DevicesWidgetService::class.java)
                .putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, appWidgetId)
                .setData(Uri.parse("se-widget://$appWidgetId"))
            @Suppress("DEPRECATION")
            views.setRemoteAdapter(R.id.devices_list, service)
        }
        views.setPendingIntentTemplate(R.id.devices_list, template(context, snapshot?.token))
        return views
    }

    /** One row; its tap carries the kind and value of the navigation in a fill-in intent. */
    fun rowViews(context: Context, row: WidgetRow): RemoteViews {
        val view = RemoteViews(context.packageName, R.layout.devices_widget_row)
        view.setTextViewText(R.id.row_name, row.name)
        view.setTextViewText(R.id.row_state, row.state)
        view.setImageViewResource(R.id.row_dot, if (row.online) R.drawable.widget_dot_online else R.drawable.widget_dot_offline)
        val (kind, value) = row.target.wire()
        view.setOnClickFillInIntent(
            R.id.row_root,
            Intent().putExtra(BridgeProtocol.NAV_EXTRA_KIND, kind).putExtra(BridgeProtocol.NAV_EXTRA_VALUE, value),
        )
        return view
    }

    /**
     * The template of the rows: an explicit intent to the app's activity with the navigation action and the token of the
     * cache. Without a token the extra is missing and the app drops the intent (nothing opens), as U40-16 says.
     */
    fun template(context: Context, token: String?): PendingIntent {
        val intent = Intent(BridgeProtocol.NAV_ACTION)
            .setComponent(ComponentName(context.packageName, MAIN_ACTIVITY))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        if (token != null) intent.putExtra(BridgeProtocol.NAV_EXTRA_TOKEN, token)
        return PendingIntent.getActivity(context, 0, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE)
    }

    private fun launchApp(context: Context): PendingIntent {
        val intent = context.packageManager.getLaunchIntentForPackage(context.packageName) ?: Intent()
        return PendingIntent.getActivity(context, 1, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    }
}
