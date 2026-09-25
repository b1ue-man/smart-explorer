package app.smartexplorer.android.ui.sync

import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.MainTab
import app.smartexplorer.android.ui.NavRequest
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** A new job to open in the editor, prefilled from a finished mirror run (spec F14). */
data class JobPrefill(val source: String, val target: String)

/**
 * Hand-off to the Sync tab from anywhere (e.g. the mirror dialog in "Dateien"): stores the
 * prefill and switches to the Sync tab, whose screen opens the editor and calls [consume].
 */
object SyncLaunch {
    private val pendingFlow = MutableStateFlow<JobPrefill?>(null)
    val pending: StateFlow<JobPrefill?> = pendingFlow.asStateFlow()

    fun newJob(source: String, target: String) {
        pendingFlow.value = JobPrefill(source, target)
        AppNav.send(NavRequest.SelectTab(MainTab.Sync))
    }

    fun consume(prefill: JobPrefill) {
        pendingFlow.compareAndSet(prefill, null)
    }
}
