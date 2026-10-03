package app.smartexplorer.android.ui.share

import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.smartexplorer.android.api.EndpointRemoval
import app.smartexplorer.android.api.ExecJob
import app.smartexplorer.android.api.ExecJobs
import app.smartexplorer.android.api.ExecTarget
import app.smartexplorer.android.api.FilesApi
import app.smartexplorer.android.api.ShareApi
import app.smartexplorer.android.api.ShareExecApi
import app.smartexplorer.android.api.ShareStatus
import app.smartexplorer.android.api.SharePolicyApi
import app.smartexplorer.android.api.ShareRequestPolicy
import app.smartexplorer.android.api.ShareSecurityApi
import app.smartexplorer.android.api.ShareServerInfo
import app.smartexplorer.android.api.UnconfirmedPairing
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Snackbars
import app.smartexplorer.android.ui.connections.RemovalReport
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** Invite code shown after creating a room or via ⋮ → Code anzeigen (`null` = none available). */
internal data class RoomCodeView(val roomName: String, val code: String?)

internal data class ShareActionFailure(val message: String, val retry: () -> Unit, val owner: Any? = null)

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
    var actionFailures by mutableStateOf<List<ShareActionFailure>>(emptyList())
        private set
    var securityError by mutableStateOf<String?>(null)
        private set
    var serverInfo by mutableStateOf<ShareServerInfo?>(null)
        private set
    var requestPolicy by mutableStateOf<ShareRequestPolicy?>(null)
        private set
    var unconfirmed by mutableStateOf<List<UnconfirmedPairing>>(emptyList())
        private set
    var pairingsKnown by mutableStateOf(false)
        private set
    private var metadataAt = 0L

    /** Commands in both directions (`share.execJobs`), loaded with the status while the worker runs. */
    var execJobs by mutableStateOf<ExecJobs?>(null)
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
    fun act(failure: String, success: String? = null, owner: Any? = null, onResult: ((String?) -> Unit)? = null, block: suspend () -> Unit) {
        var ownFailure: ShareActionFailure? = null
        var attempting = false
        fun attempt() {
            if (attempting) return
            attempting = true
            viewModelScope.launch {
                running++
                try {
                    block()
                    actionFailures = actionFailures.filterNot { it === ownFailure || (owner != null && it.owner === owner) }
                    success?.let { Snackbars.show(it) }
                    onResult?.invoke(null)
                } catch (e: CoreException) {
                    val message = "$failure: ${e.message ?: e.kind}"
                    val previous = ownFailure
                    val recorded = ShareActionFailure(message, ::attempt, owner)
                    ownFailure = recorded
                    actionFailures = actionFailures.filterNot { it === previous || (owner != null && it.owner === owner) } + recorded
                    onResult?.invoke(message)
                    Snackbars.show(message)
                } finally {
                    running--
                    attempting = false
                    metadataAt = 0L
                    reload()
                }
            }
        }
        attempt()
    }

    fun dismissActionFailure(failure: ShareActionFailure) {
        actionFailures = actionFailures.filterNot { it === failure }
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
    fun sendFiles(sources: List<String>, targetDir: String, onShow: () -> Unit) = act("Nicht gesendet") {
        FilesApi.transfer(sources, targetDir, mode = "copy", conflict = "keepBoth")
        val what = if (sources.size == 1) "1 Datei" else "${sources.size} Dateien"
        Snackbars.show("Senden gestartet: $what", "Anzeigen", onShow)
    }

    /**
     * `share.exec` on the view model's scope, so a dialog closed while the call runs cannot lose
     * the new task: once the id is known, [cancelNow] `true` cancels it right away. [onStarted]
     * gets the task id or the failure text.
     */
    fun startExec(
        location: String,
        command: String,
        shell: Boolean,
        timeoutSecs: Int,
        cancelNow: () -> Boolean,
        onStarted: (taskId: String?, failure: String?) -> Unit,
    ) {
        viewModelScope.launch {
            val id = try {
                ShareApi.exec(location, command, shell, timeoutSecs)
            } catch (e: CoreException) {
                onStarted(null, e.message ?: e.kind)
                return@launch
            }
            if (cancelNow()) {
                act("Nicht abgebrochen") { FilesApi.cancelTask(id) }
            }
            onStarted(id, null)
        }
    }

    /** Allows (after the warning) or revokes commands of [target] on this phone. */
    fun setExec(target: ExecTarget, enabled: Boolean) {
        val name = target.name.ifBlank { "Gerät" }
        act(
            if (enabled) "Befehle nicht erlaubt" else "Befehle nicht entzogen",
            if (enabled) "Befehle von $name erlaubt" else "Befehle von $name entzogen",
        ) { ShareExecApi.setExec(target.targetKey, enabled) }
    }

    /** [Stopp] of a command another device runs on this phone. */
    fun stopExec(job: ExecJob) = act("Befehl nicht gestoppt") { ShareExecApi.cancel(job) }

    internal fun report(name: String, result: EndpointRemoval) {
        removal = RemovalReport(name, result)
    }

    private suspend fun load() {
        loading = true
        try {
            val current = ShareApi.status()
            loadPolicyAndSecurity(current)
            status = current
            loadError = null
            execJobs = if (current.running) loadExecJobs() else null
        } catch (e: CoreException) {
            loadError = e.message ?: e.kind
        } finally {
            loading = false
        }
    }

    /** Notices remain until the core confirms resolution; a failed auxiliary load keeps them. */
    private suspend fun loadPolicyAndSecurity(current: ShareStatus) {
        var pairingLoaded = false
        try {
            unconfirmed = ShareSecurityApi.unconfirmedPairings()
            pairingLoaded = true
            pairingsKnown = true
            val now = System.currentTimeMillis()
            if (now - metadataAt >= 5_000L || serverInfo?.server.orEmpty() != current.server.orEmpty()) {
                serverInfo = ShareSecurityApi.serverInfo()
                requestPolicy = SharePolicyApi.policy()
                metadataAt = now
            }
            securityError = null
        } catch (e: CoreException) {
            // If the pairing query succeeded, unrelated server/policy errors do not undo that fact.
            pairingsKnown = pairingLoaded
            securityError = "Share-Sicherheit nicht geladen: ${e.message ?: e.kind}"
        }
    }

    /** A failed job list only leaves the commands out; the status itself loaded. */
    private suspend fun loadExecJobs(): ExecJobs? = try {
        ShareExecApi.jobs()
    } catch (e: CoreException) {
        Log.w(TAG, "share.execJobs failed: ${e.kind}: ${e.message}")
        null
    }

    private companion object {
        const val TAG = "SmartExplorerShare"
    }
}
