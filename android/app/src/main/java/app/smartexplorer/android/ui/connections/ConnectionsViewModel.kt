package app.smartexplorer.android.ui.connections

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.smartexplorer.android.api.ConnApi
import app.smartexplorer.android.api.Connection
import app.smartexplorer.android.api.EndpointRemoval
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.ui.common.Snackbars
import kotlinx.coroutines.launch

/** Result line of [Testen] in the form; [ok] colors it. */
internal data class TestOutcome(val ok: Boolean, val text: String)

/** Open connection form; secrets stay in memory only (never in saved UI state). */
internal class ConnectionFormState(initial: ConnectionDraft) {
    var draft by mutableStateOf(initial)
    var password by mutableStateOf("")
    var passphrase by mutableStateOf("")
    var showErrors by mutableStateOf(false)
    var testing by mutableStateOf(false)
    var saving by mutableStateOf(false)
    var outcome by mutableStateOf<TestOutcome?>(null)

    val isEdit: Boolean
        get() = draft.id != null
}

/** What was removed with a place and which sync jobs still use it (spec F12 "Löschen"). */
internal data class RemovalReport(val name: String, val removal: EndpointRemoval)

/**
 * An unknown SFTP host key to confirm: [message] is the core's `host_key_unknown` text (type and
 * fingerprint), [retry] repeats the action that met the key once it is saved.
 */
internal data class HostKeyPrompt(val message: String, val retry: () -> Unit) {
    /** The message without the machine-readable marker at its end. */
    val text: String
        get() = message.substringBefore("[se-host-key-unknown").trim()
}

private const val HOST_KEY_UNKNOWN = "host_key_unknown"

/** State of "Mehr → Verbindungen": list, form and the actions of the row menu. */
internal class ConnectionsViewModel : ViewModel() {
    var connections by mutableStateOf<List<Connection>?>(null)
        private set
    var loadError by mutableStateOf<String?>(null)
        private set
    var loading by mutableStateOf(false)
        private set
    var form by mutableStateOf<ConnectionFormState?>(null)
        private set
    var report by mutableStateOf<RemovalReport?>(null)
    var hostKeyPrompt by mutableStateOf<HostKeyPrompt?>(null)
        private set

    /** Called whenever the page is shown (the model outlives the page). */
    fun load() {
        viewModelScope.launch {
            loading = true
            try {
                connections = ConnApi.list().sortedBy { it.label.lowercase() }
                loadError = null
            } catch (e: CoreException) {
                loadError = e.message ?: e.kind
            } finally {
                loading = false
            }
        }
    }

    fun openNew() {
        form = ConnectionFormState(ConnectionDraft())
    }

    fun openEdit(connection: Connection) {
        form = ConnectionFormState(ConnectionDraft.from(connection))
    }

    fun closeForm() {
        form = null
    }

    /** [Testen] in the form: indicator while it runs, result line below the fields. */
    fun testForm(state: ConnectionFormState) {
        state.showErrors = true
        if (state.draft.errors().isNotEmpty() || state.testing) return
        val input = state.draft.toInput(state.password, state.passphrase)
        viewModelScope.launch {
            state.testing = true
            state.outcome = try {
                TestOutcome(true, ConnApi.test(input).ifBlank { "Verbindung OK" })
            } catch (e: CoreException) {
                askHostKey(e) { testForm(state) }
                TestOutcome(false, HostKeyPrompt(e.message ?: e.kind, {}).text)
            } finally {
                state.testing = false
            }
        }
    }

    fun saveForm(state: ConnectionFormState) {
        state.showErrors = true
        if (state.draft.errors().isNotEmpty() || state.saving) return
        val input = state.draft.toInput(state.password, state.passphrase)
        viewModelScope.launch {
            state.saving = true
            try {
                val saved = ConnApi.save(input)
                if (form === state) form = null
                val name = saved.label.ifBlank { saved.host }
                Snackbars.show(
                    if (saved.warnings.isEmpty()) "Gespeichert: $name"
                    else "Gespeichert: $name · ${saved.warnings.joinToString(" · ")}"
                )
                load()
            } catch (e: CoreException) {
                askHostKey(e) { saveForm(state) }
                state.outcome = TestOutcome(false, "Nicht gespeichert: ${HostKeyPrompt(e.message ?: e.kind, {}).text}")
            } finally {
                state.saving = false
            }
        }
    }

    /** ⋮ → Testen on a saved connection (stored secrets are used). */
    fun test(connection: Connection) {
        val input = ConnectionDraft.from(connection).toInput(password = "", passphrase = "")
        viewModelScope.launch {
            val text = try {
                ConnApi.test(input).ifBlank { "Verbindung OK" }
            } catch (e: CoreException) {
                askHostKey(e) { test(connection) }
                HostKeyPrompt(e.message ?: e.kind, {}).text
            }
            Snackbars.show("${connection.label}: $text")
        }
    }

    fun delete(connection: Connection) {
        viewModelScope.launch {
            try {
                val removal = ConnApi.delete(connection.id)
                report = RemovalReport(connection.label, removal)
            } catch (e: CoreException) {
                Snackbars.show("Nicht gelöscht: ${e.message ?: e.kind}")
            }
            load()
        }
    }

    fun forgetHostKey(connection: Connection) {
        viewModelScope.launch {
            try {
                ConnApi.forgetHostKey(connection.id)
                Snackbars.show("Hostschlüssel vergessen – beim nächsten Verbinden wird der neue Schlüssel zur Bestätigung angezeigt.")
            } catch (e: CoreException) {
                Snackbars.show("Nicht vergessen: ${e.message ?: e.kind}")
            }
        }
    }

    /** "Vertrauen" in the host-key dialog: saves the key, then repeats the action. */
    fun trustHostKey(prompt: HostKeyPrompt) {
        hostKeyPrompt = null
        viewModelScope.launch {
            try {
                ConnApi.trustHostKey(prompt.message)
                prompt.retry()
            } catch (e: CoreException) {
                Snackbars.show("Hostschlüssel nicht gespeichert: ${e.message ?: e.kind}")
            }
        }
    }

    fun dismissHostKey() {
        hostKeyPrompt = null
    }

    private fun askHostKey(e: CoreException, retry: () -> Unit) {
        if (e.kind == HOST_KEY_UNKNOWN) hostKeyPrompt = HostKeyPrompt(e.message ?: "", retry)
    }
}
