package app.smartexplorer.android

import android.app.Application
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.prefs.AppPrefs
import app.smartexplorer.android.service.BackgroundController
import app.smartexplorer.android.service.ExecHostNotifier
import app.smartexplorer.android.service.TaskKeeper
import app.smartexplorer.android.system.Notifications

/**
 * Process entry for the UI, services, workers and the boot receiver alike: settings, channels,
 * core start (non-blocking), task keeper, the notification for commands other devices run here and
 * the background mode.
 */
class SmartExplorerApp : Application() {
    override fun onCreate() {
        super.onCreate()
        AppPrefs.init(this)
        Notifications.ensureChannels(this)
        Core.start(this)
        TaskKeeper.start(this)
        ExecHostNotifier.start(this)
        BackgroundController.onAppStart(this)
    }
}
