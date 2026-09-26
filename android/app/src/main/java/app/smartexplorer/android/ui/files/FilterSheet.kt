package app.smartexplorer.android.ui.files

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DatePicker
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.smartexplorer.android.R
import app.smartexplorer.android.core.FilterSpec
import app.smartexplorer.android.ui.common.SeIcon
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import java.util.Locale
import kotlin.math.roundToLong

private val MODES = listOf("substring" to "Teiltext", "glob" to "Glob", "regex" to "RegEx")
private val UNITS = listOf("KB", "MB", "GB")
private val SHEET_DATE: DateTimeFormatter = DateTimeFormatter.ofPattern("dd.MM.yyyy", Locale.GERMANY)

/** Size input: text plus unit index into [UNITS]. */
private data class SizeInput(val text: String = "", val unit: Int = 1) {
    /** Bytes, `null` when empty; [valid] is false for text that is not a number ("NaN", "Infinity" included). */
    fun bytes(): Long? =
        text.trim().replace(',', '.').toDoubleOrNull()?.takeIf { it.isFinite() }?.let { (it * unitBytes(unit)).roundToLong() }

    val valid: Boolean
        get() = text.isBlank() || bytes()?.let { it >= 0 } == true

    companion object {
        fun of(bytes: Long?): SizeInput {
            if (bytes == null) return SizeInput()
            val unit = UNITS.indices.lastOrNull { bytes >= unitBytes(it) } ?: 0
            val value = bytes.toDouble() / unitBytes(unit)
            val text = if (value % 1.0 == 0.0) value.toLong().toString() else String.format(Locale.GERMANY, "%.2f", value)
            return SizeInput(text, unit)
        }

        private fun unitBytes(unit: Int): Long = 1L shl (10 * (unit + 1))
    }
}

/**
 * Sheet "Filter" (spec F5): mode, extensions, size from/to, modified from/to, files/folders,
 * hidden, problematic names only; [Zurücksetzen] [Fertig]. Changes apply on [Fertig].
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun FilterSheet(tab: BrowserTab, onDismiss: () -> Unit) {
    val initial = tab.filter.criteria
    var mode by remember { mutableStateOf(initial.mode) }
    var extensions by remember { mutableStateOf(initial.extensions) }
    var sizeMin by remember { mutableStateOf(SizeInput.of(initial.sizeMin)) }
    var sizeMax by remember { mutableStateOf(SizeInput.of(initial.sizeMax)) }
    var mtimeMin by remember { mutableStateOf(initial.mtimeMinMs) }
    var mtimeMax by remember { mutableStateOf(initial.mtimeMaxMs) }
    var files by remember { mutableStateOf(initial.files) }
    var dirs by remember { mutableStateOf(initial.dirs) }
    var hidden by remember { mutableStateOf(initial.hidden) }
    var problemOnly by remember { mutableStateOf(initial.problemOnly) }
    var picking by remember { mutableStateOf<Boolean?>(null) } // true = from, false = to

    fun reset() {
        val defaults = FilterSpec()
        mode = defaults.mode
        extensions = ""
        sizeMin = SizeInput()
        sizeMax = SizeInput()
        mtimeMin = null
        mtimeMax = null
        files = defaults.files
        dirs = defaults.dirs
        hidden = defaults.hidden
        problemOnly = defaults.problemOnly
    }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(
            modifier = Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("Filter", style = MaterialTheme.typography.titleLarge)
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                MODES.forEachIndexed { index, (value, label) ->
                    SegmentedButton(
                        selected = mode == value,
                        onClick = { mode = value },
                        shape = SegmentedButtonDefaults.itemShape(index = index, count = MODES.size),
                        label = { Text(label) },
                    )
                }
            }
            OutlinedTextField(
                value = extensions,
                onValueChange = { extensions = it },
                label = { Text("Endungen") },
                placeholder = { Text("jpg; *.heic; tar.gz") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            SizeRow("Größe von", sizeMin) { sizeMin = it }
            SizeRow("Größe bis", sizeMax) { sizeMax = it }
            DateRow("Geändert von", mtimeMin, onPick = { picking = true }, onClear = { mtimeMin = null })
            DateRow("Geändert bis", mtimeMax, onPick = { picking = false }, onClear = { mtimeMax = null })
            CheckRow("Dateien", files) { files = it }
            CheckRow("Ordner", dirs) { dirs = it }
            CheckRow("Versteckte einschließen", hidden) { hidden = it }
            CheckRow("⚠ Nur problematische Namen", problemOnly) { problemOnly = it }
            Row(
                modifier = Modifier.fillMaxWidth().padding(bottom = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                OutlinedButton(onClick = { reset() }, modifier = Modifier.weight(1f)) { Text("Zurücksetzen") }
                Button(
                    enabled = sizeMin.valid && sizeMax.valid,
                    onClick = {
                        tab.filter.criteria = FilterSpec(
                            mode = mode,
                            extensions = extensions.trim(),
                            sizeMin = sizeMin.bytes(),
                            sizeMax = sizeMax.bytes(),
                            mtimeMinMs = mtimeMin,
                            mtimeMaxMs = mtimeMax,
                            files = files,
                            dirs = dirs,
                            hidden = hidden,
                            problemOnly = problemOnly,
                        )
                        tab.filter.open = true
                        tab.filterChanged(immediate = true)
                        onDismiss()
                    },
                    modifier = Modifier.weight(1f),
                ) { Text("Fertig") }
            }
        }
    }

    picking?.let { from ->
        FilterDatePicker(
            initialMs = if (from) mtimeMin else mtimeMax,
            onPicked = { day ->
                if (from) {
                    mtimeMin = day.atStartOfDay(ZoneId.systemDefault()).toInstant().toEpochMilli()
                } else {
                    // Inclusive: up to the end of the chosen day.
                    mtimeMax = day.plusDays(1).atStartOfDay(ZoneId.systemDefault()).toInstant().toEpochMilli() - 1
                }
                picking = null
            },
            onDismiss = { picking = null },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SizeRow(label: String, input: SizeInput, onChange: (SizeInput) -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedTextField(
            value = input.text,
            onValueChange = { onChange(input.copy(text = it)) },
            label = { Text(label) },
            singleLine = true,
            isError = !input.valid,
            supportingText = if (!input.valid) {
                { Text("Zahl eingeben") }
            } else {
                null
            },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
            modifier = Modifier.weight(1f),
        )
        SingleChoiceSegmentedButtonRow(Modifier.width(180.dp)) {
            UNITS.forEachIndexed { index, unit ->
                SegmentedButton(
                    selected = input.unit == index,
                    onClick = { onChange(input.copy(unit = index)) },
                    shape = SegmentedButtonDefaults.itemShape(index = index, count = UNITS.size),
                    icon = {},
                    label = { Text(unit) },
                )
            }
        }
    }
}

@Composable
private fun DateRow(label: String, valueMs: Long?, onPick: () -> Unit, onClear: () -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(label, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
        OutlinedButton(onClick = onPick) {
            Text(valueMs?.let { SHEET_DATE.format(Instant.ofEpochMilli(it).atZone(ZoneId.systemDefault())) } ?: "beliebig")
        }
        if (valueMs != null) {
            IconButton(onClick = onClear) { SeIcon(R.drawable.ic_close, contentDescription = "$label entfernen") }
        }
    }
}

@Composable
private fun CheckRow(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth().toggleable(value = checked, onValueChange = onChange, role = Role.Checkbox),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = checked, onCheckedChange = null)
        Text(label, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.padding(start = 8.dp))
    }
}

/** Date picker; the picker works in UTC midnights, the filter in local days (ref §23). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun FilterDatePicker(initialMs: Long?, onPicked: (LocalDate) -> Unit, onDismiss: () -> Unit) {
    val initialUtc = initialMs?.let {
        Instant.ofEpochMilli(it).atZone(ZoneId.systemDefault()).toLocalDate().atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()
    }
    val state = rememberDatePickerState(initialSelectedDateMillis = initialUtc)
    DatePickerDialog(
        onDismissRequest = onDismiss,
        confirmButton = {
            TextButton(
                enabled = state.selectedDateMillis != null,
                onClick = {
                    state.selectedDateMillis?.let { onPicked(Instant.ofEpochMilli(it).atZone(ZoneOffset.UTC).toLocalDate()) }
                },
            ) { Text("OK") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Abbrechen") } },
    ) { DatePicker(state = state) }
}
