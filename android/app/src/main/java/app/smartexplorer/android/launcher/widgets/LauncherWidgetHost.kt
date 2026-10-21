package app.smartexplorer.android.launcher.widgets

import android.appwidget.AppWidgetHost
import android.appwidget.AppWidgetHostView
import android.appwidget.AppWidgetProviderInfo
import android.content.Context
import android.util.Log
import app.smartexplorer.android.launcher.LauncherSwitch

/**
 * The launcher's widget host (U38-4). The host id is [LauncherSwitch.WIDGET_HOST_ID], the same constant the
 * main process uses to free every id with `deleteHost` when the launcher is switched off (U35-9). The views it
 * makes are [LauncherWidgetHostView]s, which tell a long press from a touch meant for the widget.
 *
 * `startListening` belongs to `LauncherActivity.onStart` and `stopListening` to `onStop`; [listen] and [unlisten]
 * only keep a failing system call from ending the activity.
 */
class LauncherWidgetHost(context: Context) : AppWidgetHost(context.applicationContext, LauncherSwitch.WIDGET_HOST_ID) {
    override fun onCreateView(context: Context, appWidgetId: Int, appWidget: AppWidgetProviderInfo?): AppWidgetHostView =
        LauncherWidgetHostView(context)

    fun listen() {
        try {
            startListening()
        } catch (e: RuntimeException) {
            Log.w(TAG, "startListening failed", e)
        }
    }

    fun unlisten() {
        try {
            stopListening()
        } catch (e: RuntimeException) {
            Log.w(TAG, "stopListening failed", e)
        }
    }

    private companion object {
        const val TAG = "LauncherWidgetHost"
    }
}
