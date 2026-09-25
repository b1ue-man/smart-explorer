// material3 1.4.0: several components may still be experimental (docs/refs/compose-material3.md §0).
@file:OptIn(ExperimentalMaterial3Api::class)

package app.smartexplorer.android.ui.sync

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.api.SyncChoice
import app.smartexplorer.android.api.SyncJob
import app.smartexplorer.android.ui.common.ConfirmDialog
import app.smartexplorer.android.ui.common.ErrorCard
import app.smartexplorer.android.ui.common.LoadingBar
import app.smartexplorer.android.ui.common.SeIcon
import app.smartexplorer.android.ui.picker.LocationPickerDialog

private val WEEKDAYS = listOf("Mo", "Di", "Mi", "Do", "Fr", "Sa", "So")

/**
 * Full-screen job editor (spec F15). Every choice list comes from `sync.options`; checks run
 * locally, then through the desktop validation, and errors appear at their field.
 */
@Composable
internal fun JobEditorScreen(draft: JobDraft, onSave: () -> Unit, onClose: () -> Unit) {
    var confirmDiscard by remember { mutableStateOf(false) }
    var picking by rememberSaveable { mutableStateOf<String?>(null) }
    val requestClose: () -> Unit = {
        if (draft.dirty) {
            confirmDiscard = true
        } else {
            onClose()
        }
    }
    BackHandler(onBack = requestClose)

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(if (draft.isNew) "Neuer Sync-Job" else "Job bearbeiten") },
                navigationIcon = {
                    IconButton(onClick = requestClose) { SeIcon(R.drawable.ic_close, contentDescription = "Schließen") }
                },
                actions = {
                    TextButton(onClick = onSave, enabled = !draft.saving) { Text("Speichern") }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .imePadding()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp),
        ) {
            LoadingBar(draft.saving)
            draft.generalErrors.forEach { message ->
                ErrorCard(message, Modifier.padding(vertical = 8.dp), title = "Nicht gespeichert")
            }
            FormTextField("Name", draft.name, { draft.name = it }, error = draft.errorFor("name"))
            MainFields(draft, onPick = { picking = it })
            TriggerFields(draft)
            SwitchRow(
                "Aktiv",
                draft.enabled,
                { draft.enabled = it },
                hint = "Inaktive Jobs laufen nicht automatisch.",
            )
            HorizontalDivider(Modifier.padding(top = 8.dp))
            AdvancedHeader(draft.showAdvanced) { draft.showAdvanced = !draft.showAdvanced }
            if (draft.showAdvanced) AdvancedFields(draft)
            Spacer(Modifier.height(24.dp))
        }
    }

    picking?.let { side ->
        val isA = side == SIDE_A
        LocationPickerDialog(
            title = if (isA) "Seite A wählen" else "Seite B wählen",
            initialLocation = (if (isA) draft.source else draft.target).ifBlank { null },
            confirmLabel = "Auswählen",
            onPick = { location ->
                if (isA) {
                    draft.source = location
                } else {
                    draft.target = location
                }
                picking = null
            },
            onDismiss = { picking = null },
        )
    }
    if (confirmDiscard) {
        ConfirmDialog(
            title = "Änderungen verwerfen?",
            message = "Der Job wurde nicht gespeichert.",
            confirmLabel = "Verwerfen",
            onConfirm = {
                confirmDiscard = false
                onClose()
            },
            onDismiss = { confirmDiscard = false },
            destructive = true,
        )
    }
}

private const val SIDE_A = "a"
private const val SIDE_B = "b"

@Composable
private fun MainFields(draft: JobDraft, onPick: (String) -> Unit) {
    val volumes = rememberVolumes()
    LocationField("Seite A", draft.source, volumes, onChoose = { onPick(SIDE_A) }, error = draft.errorFor("source"))
    LocationField("Seite B", draft.target, volumes, onChoose = { onPick(SIDE_B) }, error = draft.errorFor("target"))
    SectionTitle("Richtung")
    RadioGroup(draft.options.directions, draft.direction, { draft.direction = it })
    draft.errorFor("direction")?.let { ErrorText(it) }
}

@Composable
private fun TriggerFields(draft: JobDraft) {
    SectionTitle("Auslöser")
    // "Bei Geräte-/USB-Anschluss" never fires on Android (as on Linux): hidden unless already set.
    val triggers = draft.options.triggers.filter { it.value != SyncJob.TRIGGER_CONNECT || it.value == draft.trigger }
    RadioGroup(triggers, draft.trigger, { draft.trigger = it })
    draft.errorFor("trigger")?.let { ErrorText(it) }
    Column(Modifier.padding(start = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        when (draft.trigger) {
            SyncJob.TRIGGER_INTERVAL -> FormTextField(
                "Intervall (Minuten)",
                draft.intervalMin,
                { draft.intervalMin = it },
                error = draft.errorFor("intervalMin"),
                hint = "Im Modus „Periodisch“ prüft Android frühestens alle 15 Minuten.",
                number = true,
            )
            SyncJob.TRIGGER_CALENDAR -> CalendarFields(draft)
            SyncJob.TRIGGER_REALTIME -> {
                FormTextField(
                    "Wartezeit nach Änderungen (Sekunden)",
                    draft.rtDebounceSecs,
                    { draft.rtDebounceSecs = it },
                    error = draft.errorFor("rtDebounceSecs"),
                    number = true,
                )
                HintText(
                    "Echtzeit läuft im Dauerbetrieb oder bei geöffneter App; im Modus „Periodisch“ " +
                        "einmal je Hintergrundlauf.",
                )
            }
            SyncJob.TRIGGER_STARTUP -> HintText("Läuft einmal je Gerätestart beim ersten Hintergrundlauf.")
            SyncJob.TRIGGER_CONNECT -> ErrorText(
                "„Bei Anschluss“ wird auf Android nicht unterstützt – bitte einen anderen Auslöser wählen.",
            )
            else -> Unit
        }
    }
}

@Composable
private fun CalendarFields(draft: JobDraft) {
    ChoiceRow("Zeitplan", draft.options.calendarKinds, draft.calendarKind, { draft.calendarKind = it })
    FormTextField(
        "Uhrzeit (HH:MM)",
        draft.calendarTime,
        { draft.calendarTime = it },
        error = draft.errorFor("calendar"),
    )
    if (JobDraft.isWeekly(draft.calendarKind)) {
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            WEEKDAYS.forEachIndexed { index, label ->
                val bit = 1 shl index
                FilterChip(
                    selected = draft.weekdays and bit != 0,
                    onClick = { draft.weekdays = draft.weekdays xor bit },
                    label = { Text(label) },
                )
            }
        }
    }
    if (JobDraft.isMonthly(draft.calendarKind)) {
        FormTextField("Tag im Monat (1–31)", draft.monthday, { draft.monthday = it }, number = true)
    }
    HintText("Im Modus „Periodisch“ holt der Hintergrundlauf verpasste Termine nach; pünktlich nur im Dauerbetrieb.")
}

@Composable
private fun AdvancedHeader(expanded: Boolean, onToggle: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(role = Role.Button, onClick = onToggle)
            .padding(vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text("Erweitert", style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
        SeIcon(if (expanded) R.drawable.ic_expand_less else R.drawable.ic_expand_more, contentDescription = null)
    }
}

@Composable
private fun AdvancedFields(draft: JobDraft) {
    val options = draft.options
    val twoWay = draft.direction == SyncJob.DIRECTION_BOTH
    if (twoWay) ChoiceField("Konfliktregel", options.conflicts, draft.conflict, draft.errorFor("conflict")) { draft.conflict = it }
    ChoiceField("Löschregel", options.deletePolicies, draft.deletePolicy, draft.errorFor("deletePolicy")) { draft.deletePolicy = it }
    ChoiceField("Vergleich", options.compares, draft.compare, draft.errorFor("compare")) { draft.compare = it }
    ChoiceField("Versionen", options.versionings, draft.versioning, draft.errorFor("versioning")) { draft.versioning = it }
    FormTextField(
        "Versionen behalten (Tage)",
        draft.retainDays,
        { draft.retainDays = it },
        error = draft.errorFor("retainDays"),
        number = true,
    )
    SwitchRow("Versteckte Dateien einschließen", draft.includeHidden, { draft.includeHidden = it })
    FormTextField(
        "Ignoriermuster",
        draft.ignore,
        { draft.ignore = it },
        error = draft.errorFor("ignore"),
        hint = "Ein Muster pro Zeile, z. B. *.tmp",
        singleLine = false,
        minLines = 3,
    )
    SectionTitle("Aktivzeit")
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        FormTextField("Von (HH:MM)", draft.activeFrom, { draft.activeFrom = it }, Modifier.weight(1f))
        FormTextField("Bis (HH:MM)", draft.activeTo, { draft.activeTo = it }, Modifier.weight(1f))
    }
    val windowError = draft.errorFor("activeFromMin") ?: draft.errorFor("activeToMin")
    if (windowError != null) ErrorText(windowError) else HintText("Beide leer = immer aktiv.")
    SwitchRow("Verpasste Termine nachholen", draft.catchUp, { draft.catchUp = it })
    if (!twoWay) {
        SwitchRow(
            "Dateien verschieben",
            draft.moveFiles,
            { draft.moveFiles = it },
            hint = "Übertragene Dateien werden in der Quelle entfernt.",
        )
    }
    SectionTitle("Löschschutz")
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        FormTextField(
            "Höchstens (Anzahl)",
            draft.maxDelete,
            { draft.maxDelete = it },
            Modifier.weight(1f),
            error = draft.errorFor("maxDelete"),
            number = true,
        )
        FormTextField(
            "Höchstens (%)",
            draft.maxDeletePct,
            { draft.maxDeletePct = it },
            Modifier.weight(1f),
            error = draft.errorFor("maxDeletePct"),
            number = true,
        )
    }
    HintText("Bricht einen Lauf ab, der mehr löschen würde; 0 = keine Grenze.")
    SwitchRow(
        "Papierkorb für lokale Löschungen",
        draft.useRecycleBin,
        { draft.useRecycleBin = it },
        hint = "Lokal gelöschte Dateien landen im Papierkorb statt endgültig gelöscht zu werden.",
    )
    SectionTitle("Befehle")
    val commandHint = "Nur bei Hintergrundläufen (wie am Desktop); läuft auf Android mit /system/bin/sh in der App-Sandbox."
    FormTextField("Befehl vorher", draft.runBefore, { draft.runBefore = it }, error = draft.errorFor("runBefore"), hint = commandHint)
    FormTextField("Befehl nachher", draft.runAfter, { draft.runAfter = it }, error = draft.errorFor("runAfter"), hint = commandHint)
}

@Composable
private fun ChoiceField(label: String, choices: List<SyncChoice>, selected: String, error: String?, onSelect: (String) -> Unit) {
    ChoiceRow(label, choices, selected, onSelect, error = error)
}
