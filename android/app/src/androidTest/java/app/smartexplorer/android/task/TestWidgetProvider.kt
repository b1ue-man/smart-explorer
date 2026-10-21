package app.smartexplorer.android.task

import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.Context
import android.os.Bundle
import android.os.SystemClock
import android.widget.RemoteViews
import app.smartexplorer.android.test.R
import java.io.File

/**
 * The widget of the test APK (U38-16). Every `onAppWidgetOptionsChanged` — the launcher's size messages — is
 * appended to `<filesDir>/task-report/widget-options.tsv` of the test package as
 * `id <TAB> uptime <TAB> minWidth <TAB> minHeight <TAB> maxWidth <TAB> maxHeight <TAB> sizes`, one line per call;
 * the device test reads it back with `run-as`.
 */
class TestWidgetProvider : AppWidgetProvider() {
    override fun onUpdate(context: Context, appWidgetManager: AppWidgetManager, appWidgetIds: IntArray) {
        for (id in appWidgetIds) {
            val views = RemoteViews(context.packageName, R.layout.test_widget)
            appWidgetManager.updateAppWidget(id, views)
        }
    }

    override fun onAppWidgetOptionsChanged(context: Context, appWidgetManager: AppWidgetManager, appWidgetId: Int, newOptions: Bundle) {
        val sizes = newOptions.getParcelableArrayList<android.os.Parcelable>(AppWidgetManager.OPTION_APPWIDGET_SIZES)?.size ?: 0
        val line = listOf(
            appWidgetId,
            SystemClock.uptimeMillis(),
            newOptions.getInt(AppWidgetManager.OPTION_APPWIDGET_MIN_WIDTH),
            newOptions.getInt(AppWidgetManager.OPTION_APPWIDGET_MIN_HEIGHT),
            newOptions.getInt(AppWidgetManager.OPTION_APPWIDGET_MAX_WIDTH),
            newOptions.getInt(AppWidgetManager.OPTION_APPWIDGET_MAX_HEIGHT),
            sizes,
        ).joinToString("\t")
        appendReport(context, "widget-options.tsv", line)
    }
}

/** Appends one line to a file of `<filesDir>/task-report` of the test package. */
fun appendReport(context: Context, name: String, line: String) {
    val dir = File(context.filesDir, "task-report").apply { mkdirs() }
    File(dir, name).appendText(line + "\n")
}
