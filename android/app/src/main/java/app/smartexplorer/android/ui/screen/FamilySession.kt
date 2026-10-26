package app.smartexplorer.android.ui.screen

import app.smartexplorer.android.api.FamilyApi
import app.smartexplorer.android.api.FamilyIdentity
import app.smartexplorer.android.api.FamilyListEntry
import app.smartexplorer.android.api.FamilyMember
import app.smartexplorer.android.api.DecoderOffer
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/** One exact device/family incarnation. Closing this object only disconnects. */
class FamilySession(
    target: ScreenTarget,
    val entry: FamilyListEntry,
    val deviceName: String,
    private val backend: ScreenBackend = CoreScreenBackend,
) {
    init { FamilyApi.validate(entry) }
    val target = target.copy(source = entry.source)
    val screen = ScreenSession(this.target, backend)
    val state get() = screen.state
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var opened = false
    private var takeSent = false
    val title: String get() = "${state.value.family?.snapshot?.members?.firstOrNull { it.identity == entry.identity }?.app?.ifBlank { null } ?: entry.window.app.ifBlank { "Fernfenster" }} – $deviceName"

    suspend fun open(width: Int, height: Int, fps: Int, offers: List<DecoderOffer>) {
        if (opened) return
        opened = true
        screen.open(if (target.control) "control" else "view", entry.source, width, height, fps, offers)
        scope.launch {
            state.collect { current ->
                if (target.control && current.accepted && current.control != "off" && !takeSent) {
                    takeSent = true
                    screen.call { control(it, "take") }
                }
            }
        }
    }
    fun control(on: Boolean) { screen.call { control(it, if (on) "take" else "release") } }
    fun focus(identity: FamilyIdentity) = operation(identity, "focus")
    fun closeProgram() { state.value.family?.snapshot?.let { operation(it.root, "close") } }
    fun closeMember(member: FamilyMember) = operation(member.identity, "close")
    fun resize(width: Int, height: Int) {
        if (width > 0 && height > 0) state.value.family?.snapshot?.let { operation(it.root, "resize", width, height) }
    }
    fun refresh() { screen.call { FamilyApi.refresh(it) } }
    private fun operation(identity: FamilyIdentity, operation: String, width: Int? = null, height: Int? = null) {
        screen.call { FamilyApi.operate(it, identity, operation, width, height) }
    }
    fun disconnect() { screen.close(); scope.cancel() }
}
