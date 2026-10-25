package app.smartexplorer.android.ui.viewer

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.ActivityInfo
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext

internal enum class MediaOrientation(val label: String, val request: Int) {
    Automatic("Automatisch", ActivityInfo.SCREEN_ORIENTATION_FULL_USER),
    Portrait("Hochformat", ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT),
    Landscape("Querformat", ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE),
}

private tailrec fun Context.activity(): Activity? = when (this) {
    is Activity -> this
    is ContextWrapper -> baseContext.activity()
    else -> null
}

/** A request only: large screens may ignore it, so layout always uses real constraints. */
@Composable
internal fun mediaOrientation(): Pair<MediaOrientation, (MediaOrientation) -> Unit> {
    val activity = LocalContext.current.activity()
    var selected by rememberSaveable { mutableIntStateOf(0) }
    val previous = rememberSaveable(activity) { activity?.requestedOrientation ?: ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED }
    DisposableEffect(activity) {
        onDispose { activity?.requestedOrientation = previous }
    }
    LaunchedEffect(activity, selected) {
        activity?.requestedOrientation = MediaOrientation.entries[selected].request
    }
    return MediaOrientation.entries[selected] to { selected = it.ordinal }
}
