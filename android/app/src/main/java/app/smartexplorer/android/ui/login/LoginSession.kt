package app.smartexplorer.android.ui.login

import android.graphics.Bitmap
import app.smartexplorer.android.api.LoginApi
import app.smartexplorer.android.api.LoginState
import app.smartexplorer.android.api.LoginTarget
import app.smartexplorer.android.api.LoginToken
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject

data class LoginPicture(val bitmap: Bitmap, val token: LoginToken)
private sealed interface LoginAction {
    val epoch: Long
    data class Focus(override val epoch: Long, val focused: Boolean, val control: Boolean) : LoginAction
    data class Presented(override val epoch: Long, val token: LoginToken, val presentationEpoch: Long) : LoginAction
    data class Input(override val epoch: Long, val token: LoginToken, val presentationEpoch: Long, val events: List<JsonObject>) : LoginAction
    data class Sas(override val epoch: Long, val token: LoginToken, val presentationEpoch: Long) : LoginAction
}
/** One owner, one bounded command queue and one latest bitmap. No input retry. */
class LoginSession(private val target: LoginTarget, private val user: String, private val acknowledged: Boolean) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val ended = AtomicBoolean(false)
    private val started = AtomicBoolean(false)
    private val epoch = AtomicLong(1)
    private val confirmedFocus = AtomicLong(0)
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
    fun start() { if (!started.compareAndSet(false, true)) return; scope.launch {
        try {
            id = LoginApi.open(target, user, acknowledged)
            if (ended.get()) { LoginApi.closeFast(id); return@launch }
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
            var bytes = ByteBuffer.allocateDirect(88).order(ByteOrder.LITTLE_ENDIAN)
            var colors = IntArray(0)
            while (!ended.get()) {
                val read = LoginApi.pull(id, bytes, 250)
                if (read == -1) continue
                if (read == -2) { fail(LoginApi.status(id).reason ?: "Anmeldeverbindung beendet."); return }
                require(read == -3 || read >= 88) { "Ungültiger Login-Bildkanal ($read)." }
                bytes.position(0)
                require(bytes.int == 0x3142474c) { "Ungültiger Login-Bildheader." }
                val width = bytes.int; val height = bytes.int; val length = bytes.int; val frame = bytes.long
                val hash = ByteArray(64); bytes.get(hash)
                val digest = String(hash, Charsets.US_ASCII)
                val pixels = width.toLong() * height.toLong()
                require(width in 1..4096 && height in 1..4096 && pixels <= 9_437_184 && length.toLong() == pixels * 4 && frame > 0 && digest.length == 64 && digest.all { it in "0123456789abcdef" }) { "Loginbild überschreitet die geprüften Grenzen." }
                val total = 88 + length
                if (read == -3) { require(total > bytes.capacity()); bytes = ByteBuffer.allocateDirect(total).order(ByteOrder.LITTLE_ENDIAN); continue }
                require(read == total && total <= bytes.capacity()) { "Loginbildlänge stimmt nicht." }
                if (colors.size != pixels.toInt()) colors = IntArray(pixels.toInt())
                bytes.position(88); bytes.asIntBuffer().get(colors)
                // LE BGRA becomes the documented packed ARGB integer form;
                // setPixels performs the conversion to Android's native format.
                val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
                bitmap.setPixels(colors, 0, width, 0, 0, width, height)
                publishPicture(LoginPicture(bitmap, LoginToken(frame, width, height, digest)))
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
    fun presented(token: LoginToken) {
        if (ended.get()) return
        drawn = token
        val key = "${epoch.get()}:${current.value.presentation_epoch}:${token.digest}"
        if (proof == key && current.value.input_ready) return
        proof = key; enqueue(LoginAction.Presented(epoch.get(), token, current.value.presentation_epoch))
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
    @Synchronized private fun publishPicture(next: LoginPicture) { if (!ended.get()) image.value = next }
    private fun enqueue(action: LoginAction) { if (!ended.get() && commands.trySend(action).isFailure) fail("Login-Eingabewarteschlange voll; Sitzung ohne Wiederholung beendet.") }
    private fun fail(reason: String) { current.value = current.value.copy(state = "ended", reason = reason, input_ready = false); close() }
    fun close() {
        if (ended.compareAndSet(false, true)) { epoch.incrementAndGet(); synchronized(this) { drawn = null; image.value = null; current.value = current.value.copy(state = "ended", input_ready = false) }
            try { LoginApi.closeFast(id) } finally { commands.close(); scope.cancel() } }
    }
}
