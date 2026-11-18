package app.smartexplorer.android.ui.analytics

import android.content.Context
import app.smartexplorer.android.api.AnalyzeChild
import app.smartexplorer.android.api.AnalyzeKind
import app.smartexplorer.android.api.AnalyzeNode
import app.smartexplorer.android.ui.common.Format

/**
 * "Sortieren/Kacheln nach" of the storage analysis (2026-11-18), as on the desktop: "Belegt" (the
 * space on the volume, default) or "Größe" (the sum of the file lengths). Where the result does not
 * know the occupied space (older peer, backend without the figure) "Größe" applies and the page
 * says "Belegt: unbekannt".
 */
internal enum class SizeMeasure(val key: String, val label: String) {
    Allocated("allocated", "Belegt"),
    Size("size", "Größe"),
    ;

    /** Tile area and list order of [child]. */
    fun of(child: AnalyzeChild): Long = if (this == Allocated) child.allocatedBytes else child.size

    companion object {
        /** A stored key; unknown or missing = the default "Belegt". */
        fun parse(key: String?): SizeMeasure = entries.firstOrNull { it.key == key } ?: Allocated

        /** What applies to a result: "Größe" while "Belegt" is unknown. */
        fun effective(chosen: SizeMeasure, allocatedKnown: Boolean): SizeMeasure = if (allocatedKnown) chosen else Size
    }
}

/** Why the two values differ (hint below the switch). */
internal const val MEASURE_EXPLANATION =
    "Größe: Summe der Dateilängen. Belegt: Platz, den die Dateien auf dem Datenträger tatsächlich einnehmen. " +
        "Beides weicht ab bei Platzhaltern „nur online“ (Cloud-Dateien liegen nicht auf dem Gerät), Kompression, " +
        "Sparse-Dateien (Lücken belegen nichts) und Hardlinks (mehrfach verlinkte Dateien belegen ihren Platz nur " +
        "einmal). Beim Löschen wird der belegte Platz frei."

/** "Größe 5,0 GB · Belegt 100 MB" / "Größe 5,0 GB · Belegt: unbekannt". */
internal fun sizesText(size: Long, allocated: Long, known: Boolean): String =
    if (known) "Größe ${Format.size(size)} · Belegt ${Format.size(allocated)}"
    else "Größe ${Format.size(size)} · Belegt: unbekannt"

/**
 * The sizes of one list row: both values for an entry of the result, one value for a row of
 * Android's figures (apps, estimates), which occupy what Android reports.
 */
internal fun rowSizes(child: AnalyzeChild, known: Boolean): String =
    if (child.isEstimate || child.rowKind == AnalyzeKind.App) Format.size(child.size)
    else sizesText(child.size, child.allocatedBytes, known)

/**
 * "wird frei" of duplicates: the occupied space where the search measured it
 * ([app.smartexplorer.android.api.ReclaimSummary.occupiedKnown]), else the size and "Belegt unbekannt".
 */
internal fun freedText(freed: Long, known: Boolean): String =
    if (known) "${Format.size(freed)} wird frei (belegt)" else "${Format.size(freed)} wird frei (Größe; Belegt unbekannt)"

/** Shown with "NTFS-Systemdateien" (desktop tooltip text). */
internal const val SYSTEM_FILES_HINT = "vom Dateisystem belegt, nicht löschbar"

/**
 * The analysis' notes in the order the page shows them: how the result was made ("Analyseweg: …")
 * and folders on another file system that were not counted first, then the others.
 */
internal fun orderedNotes(notes: List<String>): List<String> =
    notes.partition { prominentNote(it) }.let { (first, rest) -> first + rest }

/** A note that always stands in the result page itself, never only behind "Alle Hinweise". */
internal fun prominentNote(note: String): Boolean =
    note.startsWith("Analyseweg:") || note.startsWith("anderes Dateisystem")

/** Occupied space of the whole [node]: what the core sent, else its size. */
internal fun nodeAllocated(node: AnalyzeNode): Long = node.allocated ?: node.size

/** The person's choice, kept in private SharedPreferences (`analysis_view`, key `measure`). */
internal object AnalysisMeasurePrefs {
    private const val FILE = "analysis_view"
    private const val KEY = "measure"

    fun load(context: Context): SizeMeasure = try {
        SizeMeasure.parse(context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE).getString(KEY, null))
    } catch (e: RuntimeException) {
        SizeMeasure.Allocated
    }

    fun save(context: Context, measure: SizeMeasure) {
        try {
            context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE).edit()
                .putString(KEY, measure.key).apply()
        } catch (e: RuntimeException) {
            Unit
        }
    }
}
