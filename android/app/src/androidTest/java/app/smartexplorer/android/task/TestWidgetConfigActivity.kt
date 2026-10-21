package app.smartexplorer.android.task

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.Intent
import android.os.Bundle
import android.os.SystemClock

/**
 * The configuration activity of the test widget (U38-16): it ends with `RESULT_OK` and the widget id, and leaves a
 * line in `<filesDir>/task-report/widget-config.tsv` (`id <TAB> uptime`) — one per run, so the device test counts
 * the first configuration and each „Neu einrichten“.
 */
class TestWidgetConfigActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val id = intent.getIntExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID)
        appendReport(this, "widget-config.tsv", "$id\t${SystemClock.uptimeMillis()}")
        setResult(RESULT_OK, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id))
        finish()
    }
}
