package app.smartexplorer.android

import android.app.Application
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.launcher.LauncherSwitch
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.service.BackgroundController
import app.smartexplorer.android.service.ExecHostNotifier
import app.smartexplorer.android.service.TaskKeeper
import app.smartexplorer.android.system.Notifications
import app.smartexplorer.android.system.WakeKeeper
import app.smartexplorer.android.widget.DevicesWidgetFeed

/**
 * Process entry for the UI, services, workers, the boot receiver and the wake alarm alike:
 * settings, channels, wake locks for the core's `wake` events, core start (non-blocking), task
 * keeper, the notification for commands other devices run here, the background mode and (only when the widget part of the
 * launcher is on) the feed of the devices widget.
 *
 * The launcher process (`<package>:launcher`, DSK-M9) runs none of these steps: it has no Rust core
 * and no settings of the main process (U35-11).
 */
class SmartExplorerApp : Application() {
    override fun onCreate() {
        super.onCreate()
        if (Application.getProcessName() == packageName + LauncherSwitch.PROCESS_SUFFIX) return
        AppPrefs.init(this)
        Notifications.ensureChannels(this)
        WakeKeeper.init(this)
        Core.start(this)
        TaskKeeper.start(this)
        ExecHostNotifier.start(this)
        BackgroundController.onAppStart(this)
        // DSK-M13 (U40-17): only with the widget part on; otherwise it returns at once and nothing runs.
        DevicesWidgetFeed.startIfEnabled(this)
    }
}
