package app.smartexplorer.android.launcher

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.pm.LauncherApps
import android.graphics.Color
import android.os.Bundle
import android.os.Handler
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.runtime.snapshotFlow
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import app.smartexplorer.android.launcher.theme.LauncherTheme
import app.smartexplorer.android.launcher.ui.HomeScreen
import app.smartexplorer.android.launcher.widgets.REQ_WIDGET_BIND
import app.smartexplorer.android.launcher.widgets.REQ_WIDGET_CONFIGURE
import app.smartexplorer.android.launcher.widgets.WidgetStep
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch

/**
 * The se launcher's home screen (DSK-M9 frame, DSK-M10 content). It runs in the process `:launcher`, which
 * starts no Rust core and never calls `AppNav.token` (U35-11, U35-16), and is switched on only by the person
 * ([LauncherSwitch]).
 *
 * The Home key arrives in `onNewIntent` (`singleTask`) and sets the screen back (P22). „Zurück“ closes the topmost
 * layer — folder, app list, edit sheet, menu, search — and otherwise does nothing: a home screen never finishes.
 *
 * The widget host listens between `onStart` and `onStop` (U38-4). The system dialogs of a widget — the bind approval and
 * the provider's configuration — start from here, and their results arrive in [onActivityResult] (U38-7).
 */
class LauncherActivity : ComponentActivity() {
    private val viewModel: LauncherViewModel by viewModels()

    private val back = object : OnBackPressedCallback(false) {
        override fun handleOnBackPressed() {
            viewModel.closeTopLayer()
        }
    }

    private val launcherApps: LauncherApps by lazy { getSystemService(LauncherApps::class.java) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge(
            SystemBarStyle.auto(Color.TRANSPARENT, Color.TRANSPARENT),
            SystemBarStyle.auto(Color.TRANSPARENT, Color.TRANSPARENT),
        )
        onBackPressedDispatcher.addCallback(this, back)
        lifecycleScope.launch {
            snapshotFlow { viewModel.hasOpenLayer }.collect { back.isEnabled = it }
        }
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                snapshotFlow { viewModel.widgetStep }.filterNotNull().collect {
                    startWidgetStep(it)
                    viewModel.clearWidgetStep()
                }
            }
        }
        setContent {
            LauncherTheme { HomeScreen(viewModel) }
        }
    }

    private fun startWidgetStep(step: WidgetStep) {
        when (step) {
            is WidgetStep.AskBind -> {
                @Suppress("DEPRECATION")
                startActivityForResult(step.intent, REQ_WIDGET_BIND)
            }
            is WidgetStep.AskConfigure -> try {
                viewModel.widgetHost.startAppWidgetConfigureActivityForResult(this, step.pending.id, 0, REQ_WIDGET_CONFIGURE, null)
            } catch (_: ActivityNotFoundException) {
                viewModel.onWidgetResult(REQ_WIDGET_CONFIGURE, Activity.RESULT_CANCELED, null)
            }
            else -> Unit
        }
    }

    // `AppWidgetHost.startAppWidgetConfigureActivityForResult` takes a request code and reports to `onActivityResult` (U38-7).
    @Suppress("DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQ_WIDGET_BIND || requestCode == REQ_WIDGET_CONFIGURE) viewModel.onWidgetResult(requestCode, resultCode, data)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        if (intent.hasCategory(Intent.CATEGORY_HOME)) viewModel.goHome()
    }

    override fun onStart() {
        super.onStart()
        launcherApps.registerCallback(viewModel.callback, Handler(mainLooper))
        viewModel.widgetHost.listen()
        viewModel.onStart()
    }

    override fun onStop() {
        launcherApps.unregisterCallback(viewModel.callback)
        viewModel.widgetHost.unlisten()
        viewModel.onStop()
        super.onStop()
    }
}
