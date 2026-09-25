package app.smartexplorer.android.ui.files

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.smartexplorer.android.core.FilterSpec
import app.smartexplorer.android.ui.common.Format
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

/**
 * Filter row of one tab (spec F5): search text, further criteria from the filter sheet,
 * "Rekursiv" and the validation error of the core (`scan.validate`).
 */
internal class FilterState {
    /** Filter row visible (magnifier in the title bar). */
    var open by mutableStateOf(false)
    var text by mutableStateOf("")

    /** Every criterion except [text] (kept separately for the search field). */
    var criteria by mutableStateOf(FilterSpec())
    var recursive by mutableStateOf(false)

    /** Error of an invalid glob/regex; the filter stays off while set. */
    var error by mutableStateOf<String?>(null)

    /** The filter to apply, `null` when the row is closed or no criterion is set. */
    fun spec(): FilterSpec? {
        if (!open) return null
        return criteria.copy(text = text.trim()).takeIf { it.hasCriteria() }
    }

    /** Closes the row and lifts every criterion ("Lupe erneut schließt die Filterzeile"). */
    fun close() {
        open = false
        text = ""
        criteria = FilterSpec()
        recursive = false
        error = null
    }
}

/** `true` when the filter restricts anything (a mode alone without text does not). */
internal fun FilterSpec.hasCriteria(): Boolean =
    text.isNotBlank() || extensions.isNotBlank() || sizeMin != null || sizeMax != null ||
        mtimeMinMs != null || mtimeMaxMs != null || !files || !dirs || hidden || problemOnly

/** An active criterion shown as chip; [clear] returns the criteria without it. */
internal data class CriterionChip(val label: String, val clear: (FilterSpec) -> FilterSpec)

private val CHIP_DATE: DateTimeFormatter = DateTimeFormatter.ofPattern("dd.MM.yyyy", Locale.GERMANY)

private fun chipDate(epochMs: Long): String = CHIP_DATE.format(Instant.ofEpochMilli(epochMs).atZone(ZoneId.systemDefault()))

/** Chips for the active criteria of [spec] (the search text lives in the field itself). */
internal fun criterionChips(spec: FilterSpec): List<CriterionChip> {
    val chips = mutableListOf<CriterionChip>()
    if (spec.mode == "glob") chips += CriterionChip("Glob") { it.copy(mode = "substring") }
    if (spec.mode == "regex") chips += CriterionChip("RegEx") { it.copy(mode = "substring") }
    if (spec.extensions.isNotBlank()) chips += CriterionChip("Endungen: ${spec.extensions.trim()}") { it.copy(extensions = "") }
    val min = spec.sizeMin
    val max = spec.sizeMax
    if (min != null || max != null) {
        val label = when {
            min != null && max != null -> "${Format.size(min)} – ${Format.size(max)}"
            min != null -> "≥ ${Format.size(min)}"
            else -> "≤ ${Format.size(max ?: 0)}"
        }
        chips += CriterionChip(label) { it.copy(sizeMin = null, sizeMax = null) }
    }
    val from = spec.mtimeMinMs
    val to = spec.mtimeMaxMs
    if (from != null || to != null) {
        val label = when {
            from != null && to != null -> "${chipDate(from)} – ${chipDate(to)}"
            from != null -> "ab ${chipDate(from)}"
            else -> "bis ${chipDate(to ?: 0)}"
        }
        chips += CriterionChip(label) { it.copy(mtimeMinMs = null, mtimeMaxMs = null) }
    }
    when {
        !spec.files && !spec.dirs -> chips += CriterionChip("weder Dateien noch Ordner") { it.copy(files = true, dirs = true) }
        !spec.files -> chips += CriterionChip("nur Ordner") { it.copy(files = true) }
        !spec.dirs -> chips += CriterionChip("nur Dateien") { it.copy(dirs = true) }
    }
    if (spec.hidden) chips += CriterionChip("versteckte") { it.copy(hidden = false) }
    if (spec.problemOnly) chips += CriterionChip("⚠ nur problematische Namen") { it.copy(problemOnly = false) }
    return chips
}
