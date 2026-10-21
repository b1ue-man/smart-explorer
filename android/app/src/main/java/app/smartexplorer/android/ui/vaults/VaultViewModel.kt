package app.smartexplorer.android.ui.vaults

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.smartexplorer.android.api.VaultApi
import app.smartexplorer.android.api.VaultGroup
import app.smartexplorer.android.api.VaultSite
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreEvent
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.VaultChanged
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.launch

/**
 * State of the vault screens (U31-2, U31-10): the list from `vault.list`, refreshed by the `vault`
 * event (no polling), and the locks of sites while a screen of them is open. Holds no password.
 */
class VaultViewModel : ViewModel() {
    var groups by mutableStateOf<List<VaultGroup>>(emptyList())
        private set
    var loading by mutableStateOf(false)
        private set
    var error by mutableStateOf<String?>(null)
        private set

    private val lockedFlow = MutableSharedFlow<VaultChanged>(extraBufferCapacity = 8, onBufferOverflow = BufferOverflow.DROP_OLDEST)

    /** Every `locked` event; the open screens of that site return to the list (U31-10). */
    val locked: SharedFlow<VaultChanged> = lockedFlow.asSharedFlow()

    init {
        reload()
        viewModelScope.launch {
            Core.events.collect { event ->
                when (event) {
                    is VaultChanged -> {
                        if (event.state == "locked") lockedFlow.tryEmit(event)
                        reload()
                    }
                    is CoreEvent.Jobs -> reload()
                    else -> Unit
                }
            }
        }
    }

    val sites: List<VaultSite> get() = groups.flatMap { it.sites }

    fun site(id: String): VaultSite? = sites.firstOrNull { it.site == id }

    fun label(id: String): String = site(id)?.label?.ifBlank { null } ?: id

    fun reload() {
        viewModelScope.launch {
            loading = true
            try {
                groups = VaultApi.list().vaults
                error = null
            } catch (e: CoreException) {
                error = e.message
            } finally {
                loading = false
            }
        }
    }
}
