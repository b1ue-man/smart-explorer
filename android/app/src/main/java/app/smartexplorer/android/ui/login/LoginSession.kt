package app.smartexplorer.android.ui.login

import app.smartexplorer.android.api.LoginApi
import app.smartexplorer.android.api.LoginState
import app.smartexplorer.android.api.LoginTarget
import app.smartexplorer.android.api.LoginToken
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject

internal data class LoginPresentationStamp(val localEpoch: Long, val presentationEpoch: Long)
private sealed interface LoginAction {
    val epoch: Long
    data class Focus(override val epoch: Long, val focused: Boolean, val control: Boolean) : LoginAction
    data class Presented(override val epoch: Long, val token: LoginToken, val presentationEpoch: Long) : LoginAction
    data class Input(override val epoch: Long, val token: LoginToken, val presentationEpoch: Long, val events: List<JsonObject>) : LoginAction
    data class Sas(override val epoch: Long, val token: LoginToken, val presentationEpoch: Long) : LoginAction
}
/** One owner, one bounded command queue and one latest bitmap. No input retry. */
class LoginSession(private val target: LoginTarget, private val user: String, private val acknowledged: Boolean, internal val images: LoginImagePane) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val ended = AtomicBoolean(false)
    private val started = AtomicBoolean(false)
    private val epoch = AtomicLong(1)
    private val confirmedFocus = AtomicLong(0)
    private val renderers = HashSet<LoginBitmapRenderer>()
    private val commands = Channel<LoginAction>(8)
    private val current = MutableStateFlow(LoginState())
    val state = current.asStateFlow()
    private val image = MutableStateFlow<LoginPicture?>(null)
    val picture = image.asStateFlow()
    @Volatile private var id = 0L
    @Volatile private var focused = false
    @Volatile private var control = false
    @Volatile private var proof = ""
    @Volatile private var drawn: LoginToken? = null
    internal fun registerRenderer(renderer: LoginBitmapRenderer) {
        synchronized(renderers) { if (ended.get()) renderer.close() else renderers.add(renderer) }
    }
    internal fun unregisterRenderer(renderer: LoginBitmapRenderer) { synchronized(renderers) { renderers.remove(renderer) } }
    fun start() { if (!started.compareAndSet(false, true)) return; scope.launch {
        try {
            // Capture the native ID even when Close cancels the coroutine while
            // Core.request switches dispatchers. Native Open only starts its owner.
            withContext(NonCancellable) {
                val handle = images.openHandle()
                if (!ended.get()) {
                    id = LoginApi.open(target, user, acknowledged, handle)
                    if (ended.get()) LoginApi.closeFast(id)
                }
            }
            if (ended.get()) return@launch
            launch { poll() }; launch { pictures() }
            for (action in commands) {
                if (action.epoch != epoch.get() || ended.get()) continue
                when (action) {
                    is LoginAction.Focus -> { publish(LoginApi.focus(id, action.focused, action.control)); if (action.epoch == epoch.get()) confirmedFocus.set(action.epoch) }
                    is LoginAction.Presented -> LoginApi.presented(id, action.token, action.presentationEpoch)
                    is LoginAction.Input -> LoginApi.input(id, action.token, action.presentationEpoch, action.events)
                    is LoginAction.Sas -> LoginApi.sas(id, action.token, action.presentationEpoch)
                }
            }
        } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { fail(failure.message ?: "Anmeldeverbindung fehlgeschlagen.") }
    } }
    private suspend fun poll() {
        try { while (!ended.get()) {
            LoginApi.visible(id); val next = LoginApi.status(id); publish(next)
            if (next.state == "ended" || next.state == "stopping") { fail(next.reason ?: "Anmeldeverbindung beendet."); return }
            delay(100)
        } } catch (cancel: CancellationException) { throw cancel } catch (failure: Exception) { fail(failure.message ?: "Loginstatus verloren.") }
    }
    private suspend fun pictures() {
        try {
            while (!ended.get()) {
                val next = images.take(id)
                if (next == null) { delay(25); continue }
                publishPicture(next)
            }
        } catch (cancel: CancellationException) { throw cancel
        } catch (_: OutOfMemoryError) { fail("Speicher für das begrenzte Loginbild fehlt.")
        } catch (failure: Exception) { fail(failure.message ?: "Loginbild konnte nicht dargestellt werden.") }
    }
    fun focus(windowFocused: Boolean, allowControl: Boolean) {
        if (focused == windowFocused && control == allowControl) return
        focused = windowFocused; control = allowControl; val next = epoch.incrementAndGet(); proof = ""
        confirmedFocus.set(0); current.value = current.value.copy(input_ready = false)
        if (!windowFocused || !allowControl) LoginApi.releaseFast(id)
        enqueue(LoginAction.Focus(next, windowFocused, allowControl))
    }
    internal fun presentationStamp() = LoginPresentationStamp(epoch.get(), current.value.presentation_epoch)
    internal fun presented(token: LoginToken, stamp: LoginPresentationStamp) {
        if (ended.get() || !focused || stamp.localEpoch != epoch.get() || stamp.presentationEpoch != current.value.presentation_epoch) return
        drawn = token
        val key = "${stamp.localEpoch}:${stamp.presentationEpoch}:${token.digest}"
        if (proof == key && current.value.input_ready) return
        proof = key; enqueue(LoginAction.Presented(stamp.localEpoch, token, stamp.presentationEpoch))
    }
    fun input(token: LoginToken, events: List<JsonObject>) {
        if (ended.get() || !focused || !control || confirmedFocus.get() != epoch.get() || !current.value.input_ready || token != drawn || events.isEmpty()) return
        if (events.size > 256) { fail("Login-Eingabebündel ist zu groß; keine Wiederholung."); return }
        enqueue(LoginAction.Input(epoch.get(), token, current.value.presentation_epoch, events.toList()))
    }
    fun sas(token: LoginToken) { if (current.value.view?.access?.rights?.sas == true) enqueue(LoginAction.Sas(epoch.get(), token, current.value.presentation_epoch)) }
    fun drawnToken(): LoginToken? = drawn
    fun release() { epoch.incrementAndGet(); confirmedFocus.set(0); proof = ""; drawn = null; focused = false; control = false; current.value = current.value.copy(input_ready = false); LoginApi.releaseFast(id) }
    @Synchronized private fun publish(next: LoginState) {
        if (ended.get()) return
        val old = current.value
        if (next.state != "ended" && next.presentation_epoch < old.presentation_epoch) return
        if (next.presentation_epoch == old.presentation_epoch && next.frame != null && old.frame != null && next.frame.frame_id < old.frame.frame_id) return
        current.value = next
    }
    @Synchronized private fun publishPicture(next: LoginPicture) {
        if (ended.get()) { next.close(); return }
        val previous = image.value
        image.value = next
        previous?.close()
    }
    private fun enqueue(action: LoginAction) { if (!ended.get() && commands.trySend(action).isFailure) fail("Login-Eingabewarteschlange voll; Sitzung ohne Wiederholung beendet.") }
    private fun fail(reason: String) { current.value = current.value.copy(state = "ended", reason = reason, input_ready = false); close() }
    fun close() {
        if (ended.compareAndSet(false, true)) { epoch.incrementAndGet(); synchronized(this) { drawn = null; val previous = image.value; image.value = null; previous?.close(); current.value = current.value.copy(state = "ended", input_ready = false) }
            try { LoginApi.closeFast(id) } finally {
                val owned = synchronized(renderers) { renderers.toList().also { renderers.clear() } }
                owned.forEach { it.close() }
                commands.close(); scope.cancel()
            } }
    }
}
