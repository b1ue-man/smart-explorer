package app.smartexplorer.android.ui.share

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.smartexplorer.android.api.ACCESS_ALLOW
import app.smartexplorer.android.api.ACCESS_ASK
import app.smartexplorer.android.api.ACCESS_OFF
import app.smartexplorer.android.api.AccessAccount
import app.smartexplorer.android.api.AccessApproval
import app.smartexplorer.android.api.AccessChange
import app.smartexplorer.android.api.AccessProviderInfo
import app.smartexplorer.android.api.AccessView
import app.smartexplorer.android.api.CAP_COMMANDS
import app.smartexplorer.android.api.CAP_SCREEN_CONTROL
import app.smartexplorer.android.api.CAP_SCREEN_VIEW
import app.smartexplorer.android.api.CAP_TERMINAL
import app.smartexplorer.android.api.CapabilityRow
import app.smartexplorer.android.api.DECISION_ALWAYS
import app.smartexplorer.android.api.DECISION_DENY
import app.smartexplorer.android.api.DECISION_ONCE
import app.smartexplorer.android.api.EXEC_RELATION_ROOM
import app.smartexplorer.android.api.ExecTarget
import app.smartexplorer.android.api.ValidUntil
import app.smartexplorer.android.service.AccessApprovalNotifier
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.theme.SmartExplorerTheme

/** Rows when the core names none (cores before PRM-M10 send no `capabilities`). */
private val DEFAULT_ROWS = listOf(
    CapabilityRow(CAP_COMMANDS, "Befehle", "Programme und Shell-Befehle starten", usesAccount = true),
    CapabilityRow(CAP_TERMINAL, "Terminal", "Ein Terminal mit der Login-Shell öffnen", usesAccount = true),
)

/** Choices of „Zeitraum“: key, label and length (`null`: keep or no end). */
internal data class UntilChoice(val key: String, val label: String, val seconds: Long?)

internal const val UNTIL_KEEP = "keep"
internal const val UNTIL_NONE = "none"

private val UNTIL_LENGTHS = listOf(
    UntilChoice("1h", "1 Stunde", 3_600L),
    UntilChoice("8h", "8 Stunden", 8 * 3_600L),
    UntilChoice("1d", "1 Tag", 86_400L),
    UntilChoice("7d", "1 Woche", 7 * 86_400L),
)

/**
 * What the access dialog shows and saves: a decision per capability (commands, terminal and, since
 * SCR-M8, watching and controlling the screen), the list and the end.
 */
internal data class AccessDraft(
    val commands: String,
    val terminal: String,
    val programs: List<String>,
    val until: String,
    val screenView: String = ACCESS_OFF,
    val screenControl: String = ACCESS_OFF,
) {
    fun decision(key: String): String = when (key) {
        CAP_COMMANDS -> commands
        CAP_TERMINAL -> if (programs.isEmpty()) terminal else ACCESS_OFF
        CAP_SCREEN_VIEW -> screenView
        CAP_SCREEN_CONTROL -> screenControl
        else -> ACCESS_OFF
    }

    /** Controlling needs watching (C7): control on raises watching, watching off ends control. */
    fun with(key: String, decision: String): AccessDraft = when (key) {
        CAP_COMMANDS -> copy(commands = decision)
        CAP_TERMINAL -> copy(terminal = decision)
        CAP_SCREEN_VIEW -> copy(screenView = decision, screenControl = if (decision == ACCESS_OFF) ACCESS_OFF else screenControl)
        CAP_SCREEN_CONTROL -> copy(screenControl = decision, screenView = if (decision != ACCESS_OFF && screenView == ACCESS_OFF) decision else screenView)
        else -> this
    }

    /** Some Exec capability is on (`ask` or `allow`). */
    val grants: Boolean
        get() = decision(CAP_COMMANDS) != ACCESS_OFF || decision(CAP_TERMINAL) != ACCESS_OFF

    /** Watching or controlling the screen is on. */
    val screenGrants: Boolean
        get() = screenView != ACCESS_OFF || screenControl != ACCESS_OFF
}

/**
 * The dialog's start: the right as it is while it is on, else „Erlauben“ for commands and (without a
 * program list) the terminal, so allowing a device takes two actions: „Erlauben…“ and the confirm
 * button (plan 2026-10-17, C6 B-a).
 */
internal fun initialDraft(target: ExecTarget, view: AccessView?): AccessDraft {
    val active = view?.takeIf { target.enabled && !it.expired }
    val keepsEnd = view != null && view.validUntil != null && !view.expired
    return AccessDraft(
        commands = active?.decision(CAP_COMMANDS) ?: ACCESS_ALLOW,
        terminal = active?.decision(CAP_TERMINAL) ?: if (target.programs.isEmpty()) ACCESS_ALLOW else ACCESS_OFF,
        programs = target.programs,
        until = if (keepsEnd) UNTIL_KEEP else UNTIL_NONE,
        // The screen starts off: watching this phone is never on without a choice.
        screenView = active?.decision(CAP_SCREEN_VIEW) ?: ACCESS_OFF,
        screenControl = active?.decision(CAP_SCREEN_CONTROL) ?: ACCESS_OFF,
    )
}

/**
 * The `share.setAccess` change of a confirmed [draft] for the capabilities [keys] the dialog shows;
 * Android runs commands only as the app.
 */
internal fun accessChangeOf(draft: AccessDraft, nowSeconds: Long, keys: List<String> = listOf(CAP_COMMANDS, CAP_TERMINAL)): AccessChange = AccessChange(
    account = AccessAccount("own"),
    capabilities = keys.associateWith { draft.decision(it) },
    programs = draft.programs,
    validUntil = when (draft.until) {
        UNTIL_KEEP -> null
        UNTIL_NONE -> ValidUntil.None
        else -> ValidUntil.At(nowSeconds + (UNTIL_LENGTHS.firstOrNull { it.key == draft.until }?.seconds ?: 0L))
    },
)

/** „Aus“, „Fragen“, „Erlauben“. */
internal fun accessDecisionLabel(decision: String): String = when (decision) {
    ACCESS_ASK -> "Fragen"
    ACCESS_ALLOW -> "Erlauben"
    else -> "Aus"
}

/** „Befehle: erlaubt · Terminal: fragt · bis 17.10.2026 18:00“ of a device row. */
internal fun accessSummary(view: AccessView): String = buildList {
    val word = { decision: String ->
        when (decision) {
            ACCESS_ASK -> "fragt"
            ACCESS_ALLOW -> "erlaubt"
            else -> "aus"
        }
    }
    add("Befehle: ${word(view.decision(CAP_COMMANDS))}")
    add("Terminal: ${word(view.decision(CAP_TERMINAL))}")
    if (view.decision(CAP_SCREEN_VIEW) != ACCESS_OFF) add("Bildschirm ansehen: ${word(view.decision(CAP_SCREEN_VIEW))}")
    if (view.decision(CAP_SCREEN_CONTROL) != ACCESS_OFF) add("Bildschirm steuern: ${word(view.decision(CAP_SCREEN_CONTROL))}")
    view.validUntil?.let { add(if (view.expired) "abgelaufen" else "bis ${Format.dateTime(it * 1000)}") }
}.joinToString(" · ")

/** „Laptop möchte Befehle ausführen“. */
internal fun approvalTitle(approval: AccessApproval): String =
    approval.text.ifBlank { "${approval.deviceName.ifBlank { "Ein Gerät" }} möchte zugreifen" }

/**
 * What a question shows (plan C5): relation, shortened fingerprint, the right, the account, the
 * command; for an older client's terminal its 60-second limit.
 */
internal fun approvalLines(approval: AccessApproval): List<String> = buildList {
    val device = approval.deviceName.ifBlank { "Gerät" }
    val relation = if (approval.relation == EXEC_RELATION_ROOM) "Raum „${approval.room.orEmpty().ifBlank { "Raum" }}“" else "Direkt-Gerät"
    add("$device · $relation · Fingerabdruck ${shortFingerprint(approval.fingerprint)}")
    add("Recht: ${approval.capabilityLabel.ifBlank { approval.capability }}")
    approval.accountLabel?.takeIf { it.isNotBlank() }?.let { add("Konto: $it") }
    approval.command?.let { command ->
        when (command.mode) {
            "argv" -> add("Programm: " + (listOfNotNull(command.program) + command.args).joinToString(" "))
            "shell" -> add("Shell-Befehl: ${command.text.orEmpty()}")
            "login" -> add("Terminal (Login-Shell)")
        }
    }
    if (approval.olderClientTerminal) add("Älteres Smart Explorer auf $device: das Terminal wartet dort höchstens 60 Sekunden.")
    if (approval.screen) add("Nach dem Erlauben fragt Android, ob der Bildschirm geteilt werden darf.")
}

private fun shortFingerprint(fingerprint: String): String = if (fingerprint.length > 16) fingerprint.take(16) + "…" else fingerprint

/**
 * A question of another device as a dialog in the app: „Diesmal erlauben“, „Immer erlauben“,
 * „Ablehnen“. Back answers „Ablehnen“ unless [onDismiss] says otherwise (the list on the page
 * „Teilen“ only closes it, the question stays listed); it never allows. For a screen question the
 * allowing answers open the consent activity (SCR-M8): [AccessApprovalNotifier.decide] routes them
 * there, and the system dialog „Bildschirm teilen“ follows.
 */
@Composable
internal fun AccessApprovalPrompt(
    approval: AccessApproval,
    onDecide: (String) -> Unit,
    onDismiss: () -> Unit = { onDecide(DECISION_DENY) },
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(dismissOnClickOutside = false),
        title = { Text(approvalTitle(approval)) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                approvalLines(approval).forEach { line ->
                    val monospace = line.startsWith("Programm: ") || line.startsWith("Shell-Befehl: ")
                    Text(
                        line,
                        style = MaterialTheme.typography.bodyMedium,
                        fontFamily = if (monospace) FontFamily.Monospace else null,
                    )
                }
                Text(
                    if (approval.screen) {
                        "„Immer erlauben“ stellt dieses Recht für das Gerät auf „Erlauben“; Android fragt trotzdem bei jeder Sitzung."
                    } else {
                        "„Immer erlauben“ stellt dieses Recht für das Gerät auf „Erlauben“; spätere Starts fragen nicht mehr."
                    },
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        },
        confirmButton = {
            Row {
                TextButton(onClick = { onDecide(DECISION_ONCE) }) { Text("Diesmal erlauben") }
                TextButton(onClick = { onDecide(DECISION_ALWAYS) }) { Text("Immer erlauben") }
            }
        },
        dismissButton = { TextButton(onClick = { onDecide(DECISION_DENY) }) { Text("Ablehnen") } },
    )
}

/**
 * Shows the open questions one after another while the app is visible (plan C5: in the visible app a
 * dialog, else the notification); started by [AccessApprovalNotifier] on top of the visible screen
 * and finished once no question is open. Not exported.
 */
class AccessApprovalActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            val pending by AccessApprovalNotifier.pending.collectAsStateWithLifecycle()
            val approval = pending.firstOrNull()
            if (approval == null) {
                LaunchedEffect(Unit) { finish() }
            } else {
                SmartExplorerTheme {
                    AccessApprovalPrompt(
                        approval,
                        onDecide = { decision -> AccessApprovalNotifier.decide(approval.id, decision) },
                    )
                }
            }
        }
    }
}

/**
 * „Erlauben…“ / „Zugriff…“ of a device in „Befehle auf diesem Telefon“: commands and terminal each
 * „Aus“, „Fragen“ or „Erlauben“, „Zeitraum“, the program list and „Ausführen als: App-Konto“ with
 * the reason (plan C4). The confirm button names the consequence.
 */
@Composable
internal fun AccessDialog(
    target: ExecTarget,
    current: AccessView?,
    provider: AccessProviderInfo?,
    onConfirm: (AccessChange) -> Unit,
    onDismiss: () -> Unit,
) {
    val name = target.name.ifBlank { "Gerät" }
    var draft by remember(target.targetKey) { mutableStateOf(initialDraft(target, current)) }
    var editing by rememberSaveable(target.targetKey) { mutableStateOf(false) }
    var input by rememberSaveable(target.targetKey) { mutableStateOf("") }
    var inputError by remember(target.targetKey) { mutableStateOf<String?>(null) }
    val rows = provider?.capabilities?.takeIf { it.isNotEmpty() } ?: DEFAULT_ROWS
    val ownLabel = provider?.userLabel?.takeIf { it.isNotBlank() }
    val endChoices = buildList {
        current?.takeIf { !it.expired }?.validUntil?.let { add(UntilChoice(UNTIL_KEEP, "bis ${Format.dateTime(it * 1000)}", null)) }
        add(UntilChoice(UNTIL_NONE, "Ohne Ende", null))
        addAll(UNTIL_LENGTHS)
    }
    fun addProgram() {
        val error = programInputError(draft.programs, input)
        inputError = error
        if (error == null) {
            draft = draft.copy(programs = draft.programs + input.trim())
            input = ""
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Zugriff für „$name“") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("${execRelationText(target)} · Fingerabdruck ${target.fingerprint}", style = MaterialTheme.typography.bodySmall)
                rows.forEach { row ->
                    val locked = row.key == CAP_TERMINAL && draft.programs.isNotEmpty()
                    Text(row.label, style = MaterialTheme.typography.labelLarge)
                    if (row.description.isNotBlank()) Text(row.description, style = MaterialTheme.typography.bodySmall)
                    Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        listOf(ACCESS_OFF, ACCESS_ASK, ACCESS_ALLOW).forEach { decision ->
                            FilterChip(
                                selected = draft.decision(row.key) == decision,
                                onClick = { draft = draft.with(row.key, decision) },
                                enabled = !locked,
                                label = { Text(accessDecisionLabel(decision)) },
                            )
                        }
                    }
                    if (locked) Text("Mit einer Programmliste gibt es kein Terminal.", style = MaterialTheme.typography.bodySmall)
                }
                Text("Bei „Fragen“ bestätigst du jeden Start auf diesem Telefon.", style = MaterialTheme.typography.bodySmall)
                Text("Zeitraum", style = MaterialTheme.typography.labelLarge)
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    endChoices.forEach { choice ->
                        FilterChip(
                            selected = draft.until == choice.key,
                            onClick = { draft = draft.copy(until = choice.key) },
                            label = { Text(choice.label) },
                        )
                    }
                }
                Text("Programme: ${execProgramsText(draft.programs)}", style = MaterialTheme.typography.labelLarge)
                if (!editing) {
                    TextButton(onClick = { editing = true }) { Text("Programme ändern…") }
                } else {
                    draft.programs.forEach { program ->
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(program, fontFamily = FontFamily.Monospace, modifier = Modifier.weight(1f))
                            TextButton(onClick = { draft = draft.copy(programs = draft.programs - program) }) { Text("Entfernen") }
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
                        keyboardActions = KeyboardActions(onDone = { addProgram() }),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Row {
                        TextButton(onClick = { addProgram() }) { Text("Hinzufügen") }
                        if (draft.programs.isNotEmpty()) TextButton(onClick = { draft = draft.copy(programs = emptyList()) }) { Text("Volle Shell") }
                    }
                    Text(EXEC_PROGRAM_RULES, style = MaterialTheme.typography.bodySmall)
                }
                Text(
                    "Ausführen als: App-Konto" + ownLabel?.let { " ($it)" }.orEmpty(),
                    style = MaterialTheme.typography.labelLarge,
                )
                Text(
                    provider?.otherAccountsNote?.takeIf { it.isNotBlank() } ?: "App-Konto von Smart Explorer.",
                    style = MaterialTheme.typography.bodySmall,
                )
                if (draft.grants) {
                    Text(
                        "„$name“ kann danach " +
                            (if (draft.programs.isEmpty()) "Programme, Shell-Befehle und ein Terminal" else "nur ${draft.programs.joinToString(", ")}") +
                            " mit den Rechten des App-Kontos starten. Was das App-Konto nicht darf, darf auch das Gerät nicht.",
                        color = MaterialTheme.colorScheme.error,
                    )
                }
                if (draft.screenGrants) {
                    Text(
                        "„$name“ kann danach den Bildschirm " +
                            (if (draft.screenControl != ACCESS_OFF) "ansehen und über die Bedienungshilfe steuern" else "ansehen") +
                            ". Android fragt bei jeder Sitzung „Bildschirm teilen“.",
                        color = MaterialTheme.colorScheme.error,
                    )
                }
                Text(
                    "Android kann Befehle im Hintergrund beenden oder anhalten; mit der App enden auch laufende Befehle. " +
                        "Der Zugriff gilt nur für genau diese Geräte-Identität und lässt sich jederzeit entziehen.",
                    style = MaterialTheme.typography.bodySmall,
                )
                if (target.enabled) {
                    Text("Laufende Befehle dieses Geräts enden, wenn die Änderung Rechte einschränkt.", style = MaterialTheme.typography.bodySmall)
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(accessChangeOf(draft, System.currentTimeMillis() / 1000, rows.map { it.key })) }) {
                Text(
                    when {
                        draft.grants -> "Als App-Konto erlauben"
                        draft.screenGrants -> "Bildschirmzugriff erlauben"
                        else -> "Zugriff ausschalten"
                    },
                )
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } },
    )
}
