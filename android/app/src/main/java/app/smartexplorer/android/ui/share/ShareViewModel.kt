package app.smartexplorer.android.ui.share

import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.smartexplorer.android.api.EndpointRemoval
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.ShareApi
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.connections.RemovalReport
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** Invite code shown after creating a room or via ⋮ → Code anzeigen (`null` = none available). */
internal data class RoomCodeView(val roomName: String, val code: String?)

/**
 * State of the "Teilen" tab: the poller's latest `share.status`, the fast poll while the page is
 * visible (`share.watch`) and the actions of spec F18. Every action reloads the status afterwards.
 */
internal class ShareViewModel : ViewModel() {
    var status by mutableStateOf<ShareStatus?>(null)
        private set
    var loadError by mutableStateOf<String?>(null)
        private set
    var loading by mutableStateOf(false)
        private set
    private var running by mutableIntStateOf(0)

    /** An action is running (indicator below the top bar). */
    val busy: Boolean
        get() = running > 0

    var roomCode by mutableStateOf<RoomCodeView?>(null)
    var removal by mutableStateOf<RemovalReport?>(null)

    // StateFlows are conflated: a burst of `share` events leads to at most one extra load, and
    // `share.watch` is sent in order with only the latest wish.
    private val reloadTicks = MutableStateFlow(0L)
    private val watchWanted = MutableStateFlow(false)

    init {
        viewModelScope.launch { reloadTicks.collect { load() } }
        viewModelScope.launch {
            watchWanted.collect { active ->
                try {
                    ShareApi.watch(active)
                } catch (e: CoreException) {
                    Log.w(TAG, "share.watch($active) failed: ${e.kind}: ${e.message}")
                }
            }
        }
    }

    fun reload() {
        reloadTicks.update { it + 1 }
    }

    /** Page visible (started and composed) → the core polls the Share worker every 300 ms. */
    fun setVisible(visible: Boolean) {
        watchWanted.value = visible
    }

    /**
     * Runs a Share action; failures end in a snackbar "[failure]: <core text>", [success] is shown
     * after it worked.
     */
    fun act(failure: String, success: String? = null, block: suspend () -> Unit) {
        viewModelScope.launch {
            running++
            try {
                block()
                success?.let { Snackbars.show(it) }
            } catch (e: CoreException) {
                Snackbars.show("$failure: ${e.message ?: e.kind}")
            } finally {
                running--
                reload()
            }
        }
    }

    fun createRoom(name: String) = act("Raum nicht erstellt") {
        val created = ShareApi.createRoom(name)
        roomCode = RoomCodeView(name, created.code)
    }

    fun showRoomCode(profileId: String, name: String) = act("Code nicht verfügbar") {
        roomCode = RoomCodeView(name, ShareApi.roomCode(profileId))
    }

    fun removeDevice(contactId: String, name: String) = act("Gerät nicht entfernt") {
        report(name, ShareApi.removeDevice(contactId))
    }

    fun removeRoom(profileId: String, name: String) = act("Raum nicht entfernt") {
        report(name, ShareApi.removeRoom(profileId))
    }

    /**
     * "Datei senden": copies [sources] into [targetDir] on the device (a transfer task, spec F8;
     * remote targets number taken names). [onShow] opens the transfers sheet from the snackbar.
     */
    fun sendFiles(sources: List<String>, targetDir: String, onShow: () -> Unit) {
        viewModelScope.launch {
            try {
                FilesApi.transfer(sources, targetDir, mode = "copy", conflict = "keepBoth")
                val what = if (sources.size == 1) "1 Datei" else "${sources.size} Dateien"
                Snackbars.show("Senden gestartet: $what", "Anzeigen", onShow)
            } catch (e: CoreException) {
                Snackbars.show("Nicht gesendet: ${e.message ?: e.kind}")
            }
        }
    }

    private fun report(name: String, result: EndpointRemoval) {
        removal = RemovalReport(name, result)
    }

    private suspend fun load() {
        loading = true
        try {
            status = ShareApi.status()
            loadError = null
        } catch (e: CoreException) {
            loadError = e.message ?: e.kind
        } finally {
            loading = false
        }
    }

    private companion object {
        const val TAG = "SmartExplorerShare"
    }
}
