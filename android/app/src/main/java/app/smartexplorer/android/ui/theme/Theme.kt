package app.smartexplorer.android.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.prefs.AppPrefs

// Fallback palette (Android 11, no dynamic color): blue primary matching the launcher icon.
private val LightColors: ColorScheme = lightColorScheme(
    primary = Color(0xFF1F5FAD),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFD5E3FF),
    onPrimaryContainer = Color(0xFF001B3C),
    secondary = Color(0xFF555F71),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFD9E3F8),
    onSecondaryContainer = Color(0xFF121C2B),
    tertiary = Color(0xFF6E5676),
    onTertiary = Color(0xFFFFFFFF),
    background = Color(0xFFFBF9FF),
    surface = Color(0xFFFBF9FF),
)

private val DarkColors: ColorScheme = darkColorScheme(
    primary = Color(0xFFA7C8FF),
    onPrimary = Color(0xFF003061),
    primaryContainer = Color(0xFF004788),
    onPrimaryContainer = Color(0xFFD5E3FF),
    secondary = Color(0xFFBDC7DC),
    onSecondary = Color(0xFF273141),
    secondaryContainer = Color(0xFF3E4758),
    onSecondaryContainer = Color(0xFFD9E3F8),
    tertiary = Color(0xFFDABDE2),
    onTertiary = Color(0xFF3D2946),
    background = Color(0xFF121318),
    surface = Color(0xFF121318),
)

/** Dark mode as chosen in the settings (`AppPrefs.theme`: system, light or dark). */
@Composable
fun isAppInDarkTheme(): Boolean {
    val theme by AppPrefs.theme.collectAsStateWithLifecycle()
    return when (theme) {
        "light" -> false
        "dark" -> true
        else -> isSystemInDarkTheme()
    }
}

/** Material 3 theme; dynamic color on Android 12+, the fallback palette above on Android 11. */
@Composable
fun SmartExplorerTheme(darkTheme: Boolean = isAppInDarkTheme(), content: @Composable () -> Unit) {
    val context = LocalContext.current
    val colorScheme = when {
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S ->
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        darkTheme -> DarkColors
        else -> LightColors
    }
    MaterialTheme(colorScheme = colorScheme, content = content)
}
