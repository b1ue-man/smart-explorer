package app.smartexplorer.android.launcher

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
import androidx.lifecycle.lifecycleScope
import app.smartexplorer.android.launcher.theme.LauncherTheme
import app.smartexplorer.android.launcher.ui.HomeScreen
import kotlinx.coroutines.launch

/**
 * The se launcher's home screen (DSK-M9 frame, DSK-M10 content). It runs in the process `:launcher`, which
 * starts no Rust core and never calls `AppNav.token` (U35-11, U35-16), and is switched on only by the person
 * ([LauncherSwitch]).
 *
 * The Home key arrives in `onNewIntent` (`singleTask`) and sets the screen back (P22). „Zurück“ closes the topmost
 * layer — folder, app list, edit sheet, menu, search — and otherwise does nothing: a home screen never finishes.
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
        setContent {
            LauncherTheme { HomeScreen(viewModel) }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        if (intent.hasCategory(Intent.CATEGORY_HOME)) viewModel.goHome()
    }

    override fun onStart() {
        super.onStart()
        launcherApps.registerCallback(viewModel.callback, Handler(mainLooper))
        viewModel.onStart()
    }

    override fun onStop() {
        launcherApps.unregisterCallback(viewModel.callback)
        viewModel.onStop()
        super.onStop()
    }
}
