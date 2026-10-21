package app.smartexplorer.android.launcher

import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import app.smartexplorer.android.launcher.theme.LauncherTheme

/**
 * The home screen (DSK-M9 frame): runs in the process `:launcher`, which starts no Rust core and never
 * calls `AppNav.token` (U35-11, U35-16). The component is off until the person switches the launcher on.
 */
class LauncherActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge(
            SystemBarStyle.auto(Color.TRANSPARENT, Color.TRANSPARENT),
            SystemBarStyle.auto(Color.TRANSPARENT, Color.TRANSPARENT),
        )
        setContent {
            LauncherTheme {
                Box(Modifier.fillMaxSize()) {
                    Text("Smart Explorer Startbildschirm", Modifier.align(Alignment.Center), color = MaterialTheme.colorScheme.onSurface)
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
    }
}
