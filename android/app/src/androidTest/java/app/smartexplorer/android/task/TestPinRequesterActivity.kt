package app.smartexplorer.android.task

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.drawable.Icon
import android.os.Bundle

/**
 * Asks the home app to pin something (U38-16): `--es kind shortcut` calls `ShortcutManager.requestPinShortcut`
 * with the shortcut `se-test-pin`, `--es kind widget` calls `AppWidgetManager.requestPinAppWidget` for the test
 * widget. The answer of the home app is the launcher's dialog; this activity ends at once.
 */
class TestPinRequesterActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        when (intent.getStringExtra("kind")) {
            "shortcut" -> {
                val shortcut = ShortcutInfo.Builder(this, SHORTCUT_ID)
                    .setShortLabel("se Test-Verknüpfung")
                    .setIcon(Icon.createWithResource(this, android.R.drawable.ic_menu_info_details))
                    .setIntent(Intent(this, TestPinRequesterActivity::class.java).setAction(Intent.ACTION_MAIN))
                    .build()
                getSystemService(ShortcutManager::class.java).requestPinShortcut(shortcut, null)
            }
            "widget" -> AppWidgetManager.getInstance(this).requestPinAppWidget(ComponentName(this, TestWidgetProvider::class.java), null, null)
        }
        finish()
    }

    companion object {
        const val SHORTCUT_ID = "se-test-pin"
    }
}
