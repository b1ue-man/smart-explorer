package app.smartexplorer.android

import android.content.ActivityNotFoundException
import android.content.Intent
import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.service.BackgroundController
import app.smartexplorer.android.system.OpenWithIntent
import app.smartexplorer.android.system.ShareIntentHandler
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.AppRoot
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.theme.SmartExplorerTheme
import app.smartexplorer.android.ui.theme.isAppInDarkTheme
import app.smartexplorer.android.update.UpdateChecker
import java.lang.ref.WeakReference

/**
 * The single activity: tabs, setup, sub-pages (Compose). Receives navigation intents from
 * notifications ([AppNav.intentFor]), SEND/SEND_MULTIPLE shares and "open with" for local folders
 * and files ([OpenWithIntent]).
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            val dark = isAppInDarkTheme()
            // System bar icons follow the in-app theme, which may differ from the system setting.
            LaunchedEffect(dark) { applySystemBars(dark) }
            LaunchedEffect(Unit) {
                // A share can create a second instance; only the latest started one opens the URL.
                Core.events.collect { event ->
                    if (event is CoreEvent.OpenUrl && front.get() === this@MainActivity) openUrl(event.url)
                }
            }
            SmartExplorerTheme(darkTheme = dark) {
                Surface(Modifier.fillMaxSize()) { AppRoot() }
            }
        }
        if (savedInstanceState == null) {
            handleIntent(intent)
            UpdateChecker.maybeCheck(applicationContext)
        }
        addOnNewIntentListener { handleIntent(it) }
    }

    override fun onStart() {
        super.onStart()
        startedCount++
        front = WeakReference(this)
        BackgroundController.onUiVisible(applicationContext, true)
    }

    override fun onStop() {
        // Another instance (a share opened in the sending app's task) may still be visible: its
        // onStart runs before this onStop. A rotation recreates the activity; it is not "leaving
        // the UI".
        startedCount--
        if (startedCount == 0 && !isChangingConfigurations) BackgroundController.onUiVisible(applicationContext, false)
        super.onStop()
    }

    private fun handleIntent(intent: Intent?) {
        if (intent == null) return
        val request = AppNav.fromIntent(this, intent)
        if (request != null) {
            AppNav.send(request)
            return
        }
        when (val opened = OpenWithIntent.outcome(this, intent)) {
            is OpenWithIntent.Outcome.Open -> AppNav.send(opened.request)
            OpenWithIntent.Outcome.Refused -> Snackbars.show("Dieser Ort lässt sich in Smart Explorer nicht öffnen.")
            null -> ShareIntentHandler.handle(this, intent)
        }
    }

    private fun applySystemBars(dark: Boolean) {
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.auto(Color.TRANSPARENT, Color.TRANSPARENT) { dark },
            navigationBarStyle = SystemBarStyle.auto(LIGHT_SCRIM, DARK_SCRIM) { dark },
        )
    }

    private fun openUrl(url: String) {
        try {
            startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
        } catch (e: ActivityNotFoundException) {
            Snackbars.show("Kein Browser gefunden, um die Anmeldung zu öffnen.")
        }
    }

    private companion object {
        /** Started instances (lifecycle callbacks run on the main thread only). */
        var startedCount = 0

        /** The latest started instance. */
        var front = WeakReference<MainActivity>(null)

        // Same scrims as androidx.activity's enableEdgeToEdge() defaults.
        val LIGHT_SCRIM = Color.argb(0xe6, 0xFF, 0xFF, 0xFF)
        val DARK_SCRIM = Color.argb(0x80, 0x1b, 0x1b, 0x1b)
    }
}
