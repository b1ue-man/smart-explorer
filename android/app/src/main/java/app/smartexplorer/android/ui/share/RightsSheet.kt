package app.smartexplorer.android.ui.share

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.api.ACCESS_ALLOW
import app.smartexplorer.android.api.ACCESS_ASK
import app.smartexplorer.android.api.ACCESS_OFF
import app.smartexplorer.android.api.FOLDERS_ALL
import app.smartexplorer.android.api.FOLDERS_NONE
import app.smartexplorer.android.api.FOLDERS_SELECTED
import app.smartexplorer.android.api.RIGHTS_FIELD_COMMANDS
import app.smartexplorer.android.api.RIGHTS_FIELD_FOLDERS
import app.smartexplorer.android.api.RIGHTS_FIELD_SCREEN_CONTROL
import app.smartexplorer.android.api.RIGHTS_FIELD_SCREEN_VIEW
import app.smartexplorer.android.api.RIGHTS_FIELD_TERMINAL
import app.smartexplorer.android.api.RIGHTS_FIELD_VALID_UNTIL
import app.smartexplorer.android.api.RIGHTS_FIELD_WRITE
import app.smartexplorer.android.api.RightsPresetInfo
import app.smartexplorer.android.api.ShareExport
import app.smartexplorer.android.ui.common.Format
import app.smartexplorer.android.ui.more.ToggleSetting
import app.smartexplorer.android.ui.settings.SettingsStatusRow

/**
 * The preset chips shared by every place that chooses rights in advance (the sheet, accepting a
 * request, the Direct code, a PIN offer, a new room, automatic acceptance). [noneLabel] names the
 * choice „no preset“ (`selected == null`) where it exists; `null` leaves it out.
 */
@Composable
internal fun RightsPresetChips(
    presets: List<RightsPresetInfo>,
    selected: String?,
    onSelect: (String?) -> Unit,
    modifier: Modifier = Modifier,
    noneLabel: String? = null,
    enabled: Boolean = true,
    noneSelected: Boolean = selected == null,
) {
    FlowRow(modifier.testTag("rights-presets"), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (noneLabel != null) {
            FilterChip(selected = noneSelected, enabled = enabled, onClick = { onSelect(null) }, label = { Text(noneLabel) })
        }
        presets.forEach { preset ->
            FilterChip(selected = !noneSelected && selected == preset.key, enabled = enabled, onClick = { onSelect(preset.key) }, label = { Text(preset.label) })
        }
    }
}

/** The three-way choice off / ask / allow of one capability, as chips. */
@Composable
private fun DecisionChips(selected: String?, enabled: Boolean, onSelect: (String) -> Unit) {
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        listOf(ACCESS_OFF, ACCESS_ASK, ACCESS_ALLOW).forEach { decision ->
            FilterChip(
                selected = selected == decision,
                enabled = enabled,
                onClick = { onSelect(decision) },
                label = { Text(accessDecisionLabel(decision)) },
            )
        }
    }
}

/**
 * One row of the sheet: [title], what it means, and the three choices. A right this phone cannot
 * give shows [unavailable] instead of choices; a value this phone does not know shows no chip as
 * chosen and says „Unverändert“.
 */
@Composable
private fun RightsCapabilityRow(
    title: String,
    description: String,
    field: String,
    draft: RightsDraft,
    offered: OfferedRights,
    enabled: Boolean,
    unavailable: String,
    onDraft: (RightsDraft) -> Unit,
) {
    Column(Modifier.padding(top = 8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(title, style = MaterialTheme.typography.labelLarge)
        Text(description, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (!offered.offers(field)) {
            Text(unavailable, style = MaterialTheme.typography.bodySmall)
        } else {
            DecisionChips(if (draft.knows(field)) draft.rights.decision(field) else null, enabled) { onDraft(draft.setDecision(field, it)) }
            if (!draft.knows(field)) Text("Unverändert", style = MaterialTheme.typography.bodySmall)
        }
    }
}

/** Row „Ordner“: none / all / selection with the exact folders of `exports`, and „Ordner hinzufügen“. */
@Composable
private fun FoldersRow(
    request: RightsRequest,
    draft: RightsDraft,
    folders: List<ShareExport>,
    enabled: Boolean,
    onDraft: (RightsDraft) -> Unit,
    onAddFolder: () -> Unit,
) {
    val kind = draft.rights.folders.kind.takeIf { draft.knows(RIGHTS_FIELD_FOLDERS) }
    Column(Modifier.padding(top = 8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text("Ordner", style = MaterialTheme.typography.labelLarge)
        Text(
            if (request.target.isRoom) "„Alle“ kopiert deine Direkt-Ordner von jetzt in den Raum (später hinzugefügte nicht); mit „Auswahl“ bestimmst du die Raumordner."
            else "Welche deiner freigegebenen Ordner dieses Gerät sieht.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf(FOLDERS_NONE to "Keine", FOLDERS_ALL to "Alle Ordner", FOLDERS_SELECTED to "Auswahl").forEach { (value, label) ->
                FilterChip(selected = kind == value, enabled = enabled, onClick = { onDraft(draft.setFolders(value)) }, label = { Text(label) })
            }
        }
        if (kind == null) Text("Unverändert: die bisherige Ordnerauswahl bleibt, bis du hier etwas wählst.", style = MaterialTheme.typography.bodySmall)
        if (kind == FOLDERS_SELECTED) {
            if (folders.isEmpty()) Text("Noch kein Ordner freigegeben.", style = MaterialTheme.typography.bodySmall)
            folders.forEach { folder ->
                ShareChoice(folder.label.ifBlank { folder.path } + "  ·  " + folder.path, folder.path in draft.rights.folders.paths, enabled) {
                    onDraft(draft.togglePath(folder.path))
                }
            }
        }
        // Review 2026-10-31 G5: for a room „Alle“ copies today's Direct folders; later ones are not added.
        if (kind == FOLDERS_ALL && folders.isEmpty()) {
            Text(
                if (request.target.isRoom) "Noch kein Ordner freigegeben; „Alle“ übernimmt die Direkt-Ordner von jetzt, später hinzugefügte kommen nicht von selbst in den Raum."
                else "Noch kein Ordner freigegeben; „Alle“ gilt für Ordner, die du später hinzufügst.",
                style = MaterialTheme.typography.bodySmall,
            )
        }
        if (kind != FOLDERS_NONE) TextButton(onClick = onAddFolder, enabled = enabled) { Text("Ordner hinzufügen …") }
    }
}

/** Row „Ablauf“. */
@Composable
private fun ExpiryRow(draft: RightsDraft, enabled: Boolean, onDraft: (RightsDraft) -> Unit) {
    Column(Modifier.padding(top = 8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text("Ablauf", style = MaterialTheme.typography.labelLarge)
        val choices = buildList {
            draft.keptUntil?.let { add(EXPIRY_KEEP to "bis ${Format.dateTime(it * 1000)}") }
            EXPIRY_CHOICES.forEach { add(it.key to it.label) }
        }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            choices.forEach { (key, label) ->
                FilterChip(
                    selected = draft.knows(RIGHTS_FIELD_VALID_UNTIL) && draft.expiry == key,
                    enabled = enabled,
                    onClick = { onDraft(draft.setExpiry(key)) },
                    label = { Text(label) },
                )
            }
        }
        if (!draft.knows(RIGHTS_FIELD_VALID_UNTIL)) Text("Unverändert", style = MaterialTheme.typography.bodySmall)
        // Review 2026-10-31 G2: the end binds commands, terminal and the desktop; files never expire (as on the desktop).
        Text(EXPIRY_SCOPE_TEXT, style = MaterialTheme.typography.bodySmall)
    }
}

/** What „Ablauf“ ends, word for word as the desktop window says it (`share_rights_rows_ui.rs`). */
internal const val EXPIRY_SCOPE_TEXT = "Gilt für Befehle, Terminal und Desktop, nicht für Dateien."

/**
 * The compact menu „Rechte für <Gerät/Raum>“ (plan §F, F1/F3/F4): presets on top, one row per right,
 * „Bestätigen“ and „Abbrechen“. Swiping it away or Back puts the question off ([onDismiss]); the
 * connection stays „Rechte offen“ in the lists. „Abbrechen“ ([onCancel]) keeps the connection
 * without rights. Nothing is given before „Bestätigen“.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun RightsSheet(
    request: RightsRequest,
    draft: RightsDraft,
    onDraft: (RightsDraft) -> Unit,
    presets: List<RightsPresetInfo>,
    offered: OfferedRights,
    folders: List<ShareExport>,
    accountLabel: String,
    accountNote: String,
    action: ShareActionState,
    nowSeconds: Long,
    onConfirm: () -> Unit,
    onCancel: () -> Unit,
    onDismiss: () -> Unit,
    onAddFolder: () -> Unit,
    onMore: (() -> Unit)?,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true, confirmValueChange = { !action.saving })
    var more by rememberSaveable(request.key) { mutableStateOf(false) }
    val enabled = !action.saving
    val value = draft.effective(nowSeconds)
    ModalBottomSheet(onDismissRequest = { if (!action.saving) onDismiss() }, sheetState = sheetState, modifier = Modifier.testTag("rights-sheet")) {
        Column(
            Modifier.verticalScroll(rememberScrollState()).navigationBarsPadding().padding(horizontal = 24.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(rightsTitle(request), style = MaterialTheme.typography.titleLarge)
            Text(rightsIntro(request), style = MaterialTheme.typography.bodyMedium)
            Text("Voreinstellung", style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(top = 8.dp))
            RightsPresetChips(
                presets,
                draft.matchingPreset(presets)?.key,
                onSelect = { key -> presets.firstOrNull { it.key == key }?.let { onDraft(draft.applyPreset(it, offered)) } },
                enabled = enabled,
            )
            FoldersRow(request, draft, folders, enabled, onDraft, onAddFolder)
            ToggleSetting(
                "Schreiben (inklusive Löschen und Papierkorb)",
                draft.rights.write && draft.knows(RIGHTS_FIELD_WRITE),
                { onDraft(draft.setWrite(it)) },
                enabled = enabled,
            )
            if (!draft.knows(RIGHTS_FIELD_WRITE)) Text("Schreiben: unverändert, bis du den Schalter benutzt.", style = MaterialTheme.typography.bodySmall)
            Text(
                "Schreiben gilt nur in Ordnern, die selbst schreibbar sind (Freigaben → Rechte).",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            RightsCapabilityRow(
                "Befehle",
                "Programme und Shell-Befehle als App-Konto starten. Bei „Fragen“ bestätigst du jeden Start.",
                RIGHTS_FIELD_COMMANDS, draft, offered, enabled, offered.execNote ?: "Auf diesem Telefon nicht verfügbar.", onDraft,
            )
            RightsCapabilityRow(
                "Terminal",
                "Ein Terminal mit der Login-Shell öffnen.",
                RIGHTS_FIELD_TERMINAL, draft, offered, enabled, offered.execNote ?: "Auf diesem Telefon nicht verfügbar.", onDraft,
            )
            RightsCapabilityRow(
                "Desktop sehen",
                "Den Bildschirm dieses Telefons ansehen. Android fragt bei jeder Sitzung „Bildschirm teilen“.",
                RIGHTS_FIELD_SCREEN_VIEW, draft, offered, enabled, "Dieses Telefon bietet das Ansehen des Bildschirms nicht an.", onDraft,
            )
            RightsCapabilityRow(
                "Desktop steuern",
                "Den Bildschirm über die Bedienungshilfe bedienen. Schließt Desktop sehen ein.",
                RIGHTS_FIELD_SCREEN_CONTROL, draft, offered, enabled, "Dieses Telefon bietet das Steuern des Bildschirms nicht an.", onDraft,
            )
            ExpiryRow(draft, enabled, onDraft)
            TextButton(onClick = { more = !more }) { Text(if (more) "Weniger" else "Mehr …") }
            if (more) {
                Text("Konto: $accountLabel", style = MaterialTheme.typography.labelLarge)
                Text(accountNote, style = MaterialTheme.typography.bodySmall)
                Text(
                    "Remote Desktop (RDP) einrichten: Dieses Telefon hat keinen RDP-Dienst. Der Bildschirm läuft über „Desktop sehen/steuern“; " +
                        "ein Windows- oder Linux-Gerät richtet RDP selbst ein, wenn es dir das Recht „Desktop steuern“ gibt.",
                    style = MaterialTheme.typography.bodySmall,
                )
                if (onMore != null) {
                    TextButton(onClick = onMore, enabled = enabled) { Text("Programme, TCP, Rückfreigabe in den Geräteeinstellungen …") }
                }
            }
            Text(
                "Das Gerät erhält: " + rightsSummary(value),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(top = 8.dp).testTag("rights-summary"),
            )
            SettingsStatusRow(saving = action.saving, error = action.error)
            Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp, androidx.compose.ui.Alignment.End)) {
                TextButton(onClick = onCancel, enabled = enabled) { Text("Abbrechen") }
                Button(onClick = onConfirm, enabled = enabled, modifier = Modifier.testTag("rights-confirm")) {
                    Text(if (action.error != null) "Erneut versuchen" else confirmButtonLabel(value))
                }
            }
            Text(
                if (request.pending) "Abbrechen behält die Verbindung ohne Rechte. Wegwischen stellt die Frage zurück; die Verbindung bleibt „Rechte offen“."
                else "Abbrechen ändert nichts.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(bottom = 16.dp),
            )
        }
    }
}
