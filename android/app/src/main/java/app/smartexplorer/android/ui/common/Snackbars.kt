package app.smartexplorer.android.ui.common

import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow

/** One snackbar message; [action] runs when the user taps [actionLabel]. */
data class UiMessage(
    val text: String,
    val actionLabel: String? = null,
    val action: (() -> Unit)? = null,
)

/**
 * App-wide snackbar queue: any screen or callback calls [show]; the single [AppSnackbarHost] in
 * `AppRoot` displays the messages one after another while the UI is visible.
 */
object Snackbars {
    private val flow = MutableSharedFlow<UiMessage>(
        extraBufferCapacity = 8,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    val messages: SharedFlow<UiMessage> = flow.asSharedFlow()

    fun show(text: String, actionLabel: String? = null, action: (() -> Unit)? = null) {
        flow.tryEmit(UiMessage(text, actionLabel, action))
    }
}

@Composable
fun AppSnackbarHost(hostState: SnackbarHostState, modifier: Modifier = Modifier) {
    LaunchedEffect(hostState) {
        Snackbars.messages.collect { message ->
            val result = hostState.showSnackbar(
                message = message.text,
                actionLabel = message.actionLabel,
                withDismissAction = message.actionLabel != null,
                duration = if (message.actionLabel != null) SnackbarDuration.Long else SnackbarDuration.Short,
            )
            if (result == SnackbarResult.ActionPerformed) message.action?.invoke()
        }
    }
    SnackbarHost(hostState, modifier)
}
