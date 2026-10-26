package app.smartexplorer.android.ui.login

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Checkbox
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import app.smartexplorer.android.api.LoginTarget
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

@Composable
fun LoginViewScreen(target: LoginTarget, targetUser: String, onClose: () -> Unit) {
    var attempt by remember(target, targetUser) { mutableIntStateOf(0) }
    val session = remember(target, targetUser, attempt) { LoginSession(target, targetUser, true) }
    val state by session.state.collectAsState()
    val picture by session.picture.collectAsState()
    var control by remember(session) { mutableStateOf(false) }
    var keyboard by remember(session) { mutableStateOf(false) }
    DisposableEffect(session) { session.start(); onDispose { session.release(); session.close() } }
    LifecycleEventEffect(Lifecycle.Event.ON_PAUSE) { control = false; keyboard = false; session.release(); session.close() }
    Dialog(onDismissRequest = { session.close(); onClose() }, properties = DialogProperties(usePlatformDefaultWidth = false, dismissOnClickOutside = false)) {
        Column(Modifier.fillMaxSize().navigationBarsPadding().imePadding()) {
            Text("Maschinenanmeldung · ${target.machineId}")
            Text("Maschinenpin: ${target.publicKey}")
            Text("Administrativer Zielbenutzer: $targetUser")
            Text("Status: ${state.state}${state.reason?.let { " · $it" }.orEmpty()}")
            state.notice?.let { Text(it) }
            state.view?.challenge?.let { challenge ->
                Text("Sitzung ${challenge.binding.session_id} · ${challenge.binding.desktop} · Policy ${challenge.policy_revision}")
                Text("Boot: ${challenge.boot_generation}\nSitzungsgeneration: ${challenge.binding.session_generation}\nDesktopgeneration: ${challenge.binding.desktop_generation}", maxLines = 4)
            }
            val rights = state.view?.access?.rights
            Text("Administrative Rechte: Ansicht=${rights?.screen_view == true} · Steuerung=${rights?.screen_control == true} · Anmeldung=${rights?.login == true} · Sperre=${rights?.locked == true} · UAC=${rights?.uac == true} · SAS=${rights?.sas == true}")
            Row {
                Checkbox(checked = control, enabled = state.state != "ended" && rights?.let { it.screen_control && it.login && it.screen_view } == true,
                    onCheckedChange = { control = it; if (!it) { keyboard = false; session.release() } })
                Text("Steuern")
                TextButton(enabled = control && state.input_ready, onClick = { keyboard = !keyboard }) { Text("Tastatur") }
                TextButton(enabled = rights?.let { it.sas && it.screen_control && it.login } == true && picture != null && state.state != "ended",
                    onClick = { picture?.token?.let(session::sas) }) { Text("Strg+Alt+Entf") }
            }
            Row {
                if (state.state == "ended") TextButton(onClick = { attempt++ }) { Text("Neuen Kontext starten") }
                TextButton(onClick = { session.release(); session.close() }) { Text("Beenden") }
                TextButton(onClick = { session.close(); onClose() }) { Text("Schließen") }
            }
            if (keyboard && control) LoginKeyboard(session, state.input_ready)
            key(session) {
                AndroidView(factory = { LoginCanvas(it, session) }, update = { it.update(picture.takeIf { state.state != "ended" }, control && state.state != "ended") },
                    modifier = Modifier.fillMaxWidth().weight(1f))
            }
        }
    }
}

/** IME composition is held locally until commit; no text is persisted or logged. */
@Composable
private fun LoginKeyboard(session: LoginSession, enabled: Boolean) {
    val pad = remember { "\u200B".repeat(32) }
    var field by remember(session) { mutableStateOf(TextFieldValue(pad, TextRange(pad.length))) }
    val focus = remember { FocusRequester() }; val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(session) { focus.requestFocus(); keyboard?.show() }
    fun key(name: String) = listOf(true, false).map { down -> buildJsonObject { put("type", "key"); put("key", name); put("down", down) } }
    fun send(events: List<JsonObject>) { if (enabled) session.drawnToken()?.let { session.input(it, events) } }
    BasicTextField(value = field, onValueChange = { next ->
        field = next
        if (next.composition == null) {
            if (next.text.startsWith(pad)) {
                val text = next.text.substring(pad.length)
                if (text.isNotEmpty()) {
                    val events = ArrayList<JsonObject>(); var chunk = ""
                    for (point in text.codePoints().toArray()) {
                        val nextPoint = String(Character.toChars(point))
                        if ((chunk + nextPoint).toByteArray(Charsets.UTF_8).size > 4096) { val part = chunk; events.add(buildJsonObject { put("type", "text"); put("text", part) }); chunk = "" }
                        chunk += nextPoint
                    }
                    if (chunk.isNotEmpty()) { val part = chunk; events.add(buildJsonObject { put("type", "text"); put("text", part) }) }
                    send(events)
                }
            } else if (next.text.length < pad.length) send((0 until (pad.length - next.text.length).coerceAtMost(32)).flatMap { key("backspace") })
            field = TextFieldValue(pad, TextRange(pad.length))
        }
    }, modifier = Modifier.size(1.dp).alpha(0f).focusRequester(focus),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Send),
        keyboardActions = KeyboardActions(onSend = { send(key("enter")) }))
}
