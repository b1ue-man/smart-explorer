package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import app.smartexplorer.android.api.EXEC_RUN_AS_STANDARD
import app.smartexplorer.android.api.ExecProviderInfo
import app.smartexplorer.android.api.ExecTarget
import app.smartexplorer.android.api.ShareExecApi

/** Longest program entry the core accepts (`PATH_MAX`, `ExecGrantSettings::validate`). */
private const val MAX_PROGRAM_BYTES = 4096

/**
 * The rules of `se share grants exec enable --allow-program` (CLI help), as the desktop shows them.
 */
internal const val EXEC_PROGRAM_RULES =
    "Ein Eintrag ist ein Programmname (gesucht nur im festen Systemsuchpfad dieses Telefons, nie im PATH des anderen Geräts) " +
        "oder ein absoluter Pfad. Gestartet wird nur dieselbe Datei unter demselben Namen; ein Eintrag, den dieses Telefon nicht " +
        "findet, erlaubt nichts. Das Gerät darf nur LANG, LC_* und TERM als Umgebung senden. Die Argumente bleiben frei: Shells, env, " +
        "Interpreter (zum Beispiel sh, python) und Programme, die Befehle ausführen (zum Beispiel find, xargs, awk, su), sind eine volle " +
        "Shell und gehören nicht in die Liste. Keine Sandbox."

/** Why [input] cannot join [draft] (the core's rules: bare name or absolute path, no duplicate), or `null`. */
internal fun programInputError(draft: List<String>, input: String): String? {
    val program = input.trim()
    return when {
        program.isEmpty() -> "Bitte einen Programmnamen oder absoluten Pfad eingeben."
        program in draft -> "$program steht schon in der Liste."
        program.contains('\u0000') || program.toByteArray().size > MAX_PROGRAM_BYTES -> "Ungültiger Programmname."
        program.contains('/') && !program.startsWith("/") -> "Nur ein Programmname oder ein absoluter Pfad (mit / am Anfang)."
        else -> null
    }
}

/** "Standardnutzer (Konto der App)" or the account a grant names (Android cannot switch accounts). */
internal fun execRunAsText(target: ExecTarget, provider: ExecProviderInfo): String =
    if (target.runAs == EXEC_RUN_AS_STANDARD || target.runAs.isBlank()) {
        "Standardnutzer" + provider.userLabel.takeIf { it.isNotBlank() }?.let { " ($it, Konto der App)" }.orEmpty()
    } else {
        "Konto ${target.runAs} (auf Android nicht möglich)"
    }

/** "volle Shell" or "nur uptime, df". */
internal fun execProgramsText(programs: List<String>): String =
    if (programs.isEmpty()) "volle Shell" else "nur ${programs.joinToString(", ")}"

/**
 * „Programme…“ of a device in „Befehle auf diesem Telefon“: the list a device may start (empty = full
 * shell). For an allowed device every change needs „Ich habe verstanden“ and ends its running
 * commands, as `enable` in the CLI asks for YES.
 */
@Composable
internal fun ExecProgramsDialog(target: ExecTarget, vm: ShareViewModel, onDismiss: () -> Unit) {
    val action = rememberShareAction()
    var draft by remember(target.targetKey) { mutableStateOf(target.programs) }
    var input by rememberSaveable(target.targetKey) { mutableStateOf("") }
    var inputError by rememberSaveable(target.targetKey) { mutableStateOf<String?>(null) }
    var understood by rememberSaveable(target.targetKey) { mutableStateOf(false) }
    val changed = draft != target.programs
    val name = target.name.ifBlank { "Gerät" }
    fun add() {
        val error = programInputError(draft, input)
        inputError = error
        if (error == null) {
            draft = draft + input.trim()
            input = ""
        }
    }
    ShareActionDialog(
        title = "Programme für „$name“",
        state = action,
        onDismiss = onDismiss,
        confirmLabel = "Übernehmen",
        enabled = changed && (!target.enabled || understood),
        onConfirm = {
            action.submit(vm, "Programmliste nicht gespeichert", onDismiss) {
                ShareExecApi.setExec(target.targetKey, target.enabled, draft)
            }
        },
    ) {
        Text(if (draft.isEmpty()) "Leer: volle Shell (beliebige Programme und Shell-Befehle, Terminal)." else "Nur diese Programme, ohne Shell und Terminal:")
        draft.forEach { program ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(program, fontFamily = FontFamily.Monospace, modifier = Modifier.weight(1f))
                TextButton(onClick = { draft = draft - program }) { Text("Entfernen") }
            }
        }
        OutlinedTextField(
            value = input,
            onValueChange = {
                input = it
                inputError = null
            },
            label = { Text("Programm hinzufügen") },
            placeholder = { Text("uptime oder /system/bin/df") },
            singleLine = true,
            isError = inputError != null,
            supportingText = if (inputError != null) {
                { Text(inputError.orEmpty()) }
            } else {
                null
            },
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { add() }),
            modifier = Modifier.fillMaxWidth(),
        )
        Row {
            TextButton(onClick = { add() }) { Text("Hinzufügen") }
            if (draft.isNotEmpty()) TextButton(onClick = { draft = emptyList() }) { Text("Volle Shell (Liste leeren)") }
        }
        Text(EXEC_PROGRAM_RULES, style = MaterialTheme.typography.bodySmall)
        if (changed && target.enabled) {
            Text(
                if (draft.isEmpty()) {
                    "„$name“ darf danach wieder beliebige Shell-Befehle auf diesem Telefon starten."
                } else {
                    "„$name“ darf danach nur noch ${draft.joinToString(", ")} starten."
                },
                color = MaterialTheme.colorScheme.error,
            )
            Text("Laufende Befehle dieses Geräts enden.")
            ShareChoice("Ich habe verstanden", understood) { understood = it }
        } else if (changed) {
            Text("Die Liste gilt, sobald du die Befehle dieses Geräts erlaubst.", style = MaterialTheme.typography.bodySmall)
        }
    }
}
