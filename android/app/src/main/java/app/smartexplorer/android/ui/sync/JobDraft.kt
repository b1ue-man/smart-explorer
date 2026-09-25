package app.smartexplorer.android.ui.sync

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.smartexplorer.android.api.SyncCalendar
import app.smartexplorer.android.api.SyncJob
import app.smartexplorer.android.api.SyncOptions

/**
 * Editable state of the job editor (spec F15). Numbers and times are kept as text so a
 * half-typed value does not snap back; [build] turns the draft into a job plus local field
 * errors. Error keys are the api.md field names; [errorFor] also matches snake_case keys.
 */
@Stable
internal class JobDraft(private val original: SyncJob, val options: SyncOptions) {
    val isNew: Boolean = original.id.isEmpty()

    var name by mutableStateOf(original.name)
    var source by mutableStateOf(original.source)
    var target by mutableStateOf(original.target)
    var direction by mutableStateOf(original.direction)
    var enabled by mutableStateOf(original.enabled)

    var trigger by mutableStateOf(original.trigger)
    var intervalMin by mutableStateOf(original.intervalMin.toString())
    var calendarKind by mutableStateOf(original.calendar?.kind?.takeIf { it.isNotEmpty() } ?: options.calendarKinds.firstOrNull()?.value.orEmpty())
    var calendarTime by mutableStateOf(minutesToText(original.calendar?.minuteOfDay ?: DEFAULT_CALENDAR_MINUTE))
    /** Desktop bitmask: bit0 = Mo … bit6 = So (0 = no weekday chosen). */
    var weekdays by mutableStateOf((original.calendar?.weekday ?: 0) and ALL_WEEKDAYS)
    var monthday by mutableStateOf((original.calendar?.monthday ?: 1).coerceIn(1, 31).toString())
    var rtDebounceSecs by mutableStateOf(original.rtDebounceSecs.toString())

    var conflict by mutableStateOf(original.conflict)
    var deletePolicy by mutableStateOf(original.deletePolicy)
    var compare by mutableStateOf(original.compare)
    var versioning by mutableStateOf(original.versioning)
    var retainDays by mutableStateOf(original.retainDays.toString())
    var includeHidden by mutableStateOf(original.includeHidden)
    var ignore by mutableStateOf(original.ignore.joinToString("\n"))
    var activeFrom by mutableStateOf(windowText(original.activeFromMin, original.activeToMin, original.activeFromMin))
    var activeTo by mutableStateOf(windowText(original.activeFromMin, original.activeToMin, original.activeToMin))
    var catchUp by mutableStateOf(original.catchUp)
    var moveFiles by mutableStateOf(original.moveFiles)
    var maxDelete by mutableStateOf(original.maxDelete.toString())
    var maxDeletePct by mutableStateOf(original.maxDeletePct.toString())
    var useRecycleBin by mutableStateOf(original.useRecycleBin)
    var runBefore by mutableStateOf(original.runBefore)
    var runAfter by mutableStateOf(original.runAfter)

    var showAdvanced by mutableStateOf(false)
    var saving by mutableStateOf(false)

    /** Field errors (local parsing and `sync.validate`), keyed by api.md field name. */
    var errors by mutableStateOf<Map<String, String>>(emptyMap())

    /** Errors whose key names no field shown in the editor (e.g. a save failure). */
    val generalErrors: List<String>
        get() = errors.filterKeys { key -> FIELDS.none { same(it, key) } }.values.toList()

    fun errorFor(field: String): String? = errors.entries.firstOrNull { same(it.key, field) }?.value

    /** `true` when the draft differs from the job it was opened with. */
    val dirty: Boolean
        get() {
            val (job, problems) = build()
            return problems.isNotEmpty() || job != original
        }

    /** The job as the editor shows it, plus local errors (empty = ready for `sync.validate`). */
    fun build(): Pair<SyncJob, Map<String, String>> {
        val problems = linkedMapOf<String, String>()
        val interval = if (trigger == SyncJob.TRIGGER_INTERVAL) {
            number(intervalMin, "intervalMin", 1, MAX_INTERVAL_MIN, problems) ?: original.intervalMin
        } else {
            original.intervalMin
        }
        val debounce = if (trigger == SyncJob.TRIGGER_REALTIME) {
            number(rtDebounceSecs, "rtDebounceSecs", 0, MAX_DEBOUNCE_SECS, problems) ?: original.rtDebounceSecs
        } else {
            original.rtDebounceSecs
        }
        val calendar = if (trigger == SyncJob.TRIGGER_CALENDAR) buildCalendar(problems) else original.calendar
        val (from, to) = buildWindow(problems)
        val job = original.copy(
            name = name.trim(),
            source = source,
            target = target,
            direction = direction,
            enabled = enabled,
            trigger = trigger,
            intervalMin = interval,
            calendar = calendar,
            rtDebounceSecs = debounce,
            conflict = conflict,
            deletePolicy = deletePolicy,
            compare = compare,
            versioning = versioning,
            retainDays = number(retainDays, "retainDays", 0, MAX_RETAIN_DAYS, problems) ?: original.retainDays,
            includeHidden = includeHidden,
            ignore = ignore.lines().map { it.trim() }.filter { it.isNotEmpty() },
            activeFromMin = from,
            activeToMin = to,
            catchUp = catchUp,
            moveFiles = moveFiles,
            maxDelete = number(maxDelete, "maxDelete", 0, Int.MAX_VALUE, problems) ?: original.maxDelete,
            maxDeletePct = number(maxDeletePct, "maxDeletePct", 0, 100, problems) ?: original.maxDeletePct,
            useRecycleBin = useRecycleBin,
            runBefore = runBefore.trim(),
            runAfter = runAfter.trim(),
        )
        if (source.isBlank()) problems["source"] = "Seite A wählen"
        if (target.isBlank()) problems["target"] = "Seite B wählen"
        return job to problems
    }

    private fun buildCalendar(problems: MutableMap<String, String>): SyncCalendar {
        val minute = textToMinutes(calendarTime)
        if (minute == null) problems["calendar"] = "Uhrzeit als HH:MM eingeben"
        val base = original.calendar ?: SyncCalendar()
        val day = if (isMonthly(calendarKind)) number(monthday, "calendar", 1, 31, problems) else null
        if (isWeekly(calendarKind) && weekdays == 0) problems["calendar"] = "Mindestens einen Wochentag wählen"
        return base.copy(
            kind = calendarKind,
            minuteOfDay = minute ?: base.minuteOfDay,
            weekday = if (isWeekly(calendarKind)) weekdays else base.weekday,
            monthday = day ?: base.monthday,
        )
    }

    /** Both empty = always active (stored as equal times, like the desktop). */
    private fun buildWindow(problems: MutableMap<String, String>): Pair<Int, Int> {
        if (activeFrom.isBlank() && activeTo.isBlank()) {
            val unchanged = original.activeFromMin == original.activeToMin
            return if (unchanged) original.activeFromMin to original.activeToMin else 0 to 0
        }
        val from = textToMinutes(activeFrom)
        val to = textToMinutes(activeTo)
        if (from == null || to == null) {
            problems["activeFromMin"] = "Beide Zeiten als HH:MM eingeben oder beide leer lassen"
            return original.activeFromMin to original.activeToMin
        }
        return from to to
    }

    private fun number(text: String, field: String, min: Int, max: Int, problems: MutableMap<String, String>): Int? {
        val value = text.trim().toIntOrNull()
        if (value == null || value < min || value > max) {
            problems[field] = if (max == Int.MAX_VALUE) "Ganze Zahl ab $min eingeben" else "Ganze Zahl von $min bis $max eingeben"
            return null
        }
        return value
    }

    companion object {
        private const val DEFAULT_CALENDAR_MINUTE = 9 * 60
        private const val ALL_WEEKDAYS = 0x7f
        private const val MAX_INTERVAL_MIN = 7 * 24 * 60
        private const val MAX_DEBOUNCE_SECS = 24 * 60 * 60
        private const val MAX_RETAIN_DAYS = 36_500

        /** Every field the editor shows an error under (api.md names). */
        private val FIELDS = listOf(
            "name", "source", "target", "direction", "trigger", "intervalMin", "calendar", "rtDebounceSecs",
            "conflict", "deletePolicy", "compare", "versioning", "retainDays", "ignore", "activeFromMin",
            "activeToMin", "maxDelete", "maxDeletePct", "runBefore", "runAfter",
        )

        fun isWeekly(kind: String): Boolean = kind.contains("week", ignoreCase = true)

        fun isMonthly(kind: String): Boolean = kind.contains("month", ignoreCase = true)

        /** Compares api.md field names with core keys that may be snake_case. */
        private fun same(a: String, b: String): Boolean =
            a.replace("_", "").equals(b.replace("_", ""), ignoreCase = true)

        fun minutesToText(minutes: Int): String {
            val m = Math.floorMod(minutes, 24 * 60)
            return "%02d:%02d".format(m / 60, m % 60)
        }

        /** "HH:MM" or a bare hour → minutes after midnight; `null` when unreadable. */
        fun textToMinutes(text: String): Int? {
            val trimmed = text.trim()
            val hour: Int
            val minute: Int
            if (trimmed.contains(':')) {
                hour = trimmed.substringBefore(':').trim().toIntOrNull() ?: return null
                minute = trimmed.substringAfter(':').trim().toIntOrNull() ?: return null
            } else {
                hour = trimmed.toIntOrNull() ?: return null
                minute = 0
            }
            return if (hour in 0..23 && minute in 0..59) hour * 60 + minute else null
        }

        private fun windowText(from: Int, to: Int, value: Int): String = if (from == to) "" else minutesToText(value)
    }
}
