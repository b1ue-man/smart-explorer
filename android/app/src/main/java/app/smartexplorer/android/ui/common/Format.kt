package app.smartexplorer.android.ui.common

import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

/** German display formats for sizes, rates and dates. */
object Format {
    private val LOCALE: Locale = Locale.GERMANY
    private val UNITS = listOf("B", "KB", "MB", "GB", "TB", "PB")
    private val DAY_MONTH = DateTimeFormatter.ofPattern("dd.MM.", LOCALE)
    private val DATE = DateTimeFormatter.ofPattern("dd.MM.yyyy", LOCALE)
    private val DATE_TIME = DateTimeFormatter.ofPattern("dd.MM.yyyy HH:mm", LOCALE)
    private val TIME = DateTimeFormatter.ofPattern("HH:mm", LOCALE)
    private const val NONE = "–"

    /** `0 B`, `999 B`, `3,2 KB`, `45,3 MB`, `124 MB`, `1,8 GB` (binary units). */
    fun size(bytes: Long): String {
        if (bytes < 1024) return "${bytes.coerceAtLeast(0)} B"
        var value = bytes.toDouble()
        var unit = 0
        while (value >= 1024 && unit < UNITS.lastIndex) {
            value /= 1024
            unit++
        }
        val pattern = if (value < 100) "%.1f" else "%.0f"
        return String.format(LOCALE, pattern, value) + " " + UNITS[unit]
    }

    /** Transfer rate, e.g. `4,5 MB/s`. */
    fun rate(bytesPerSecond: Long): String = size(bytesPerSecond) + "/s"

    /** `12.09.` in the current year, `12.09.2025` otherwise; `–` for unknown times. */
    fun date(epochMs: Long, nowMs: Long = System.currentTimeMillis()): String {
        if (epochMs <= 0) return NONE
        val time = zoned(epochMs)
        return if (time.year == zoned(nowMs).year) DAY_MONTH.format(time) else DATE.format(time)
    }

    /** `12.09.2026 14:03`; `–` for unknown times. */
    fun dateTime(epochMs: Long): String = if (epochMs <= 0) NONE else DATE_TIME.format(zoned(epochMs))

    /** `14:03`; `–` for unknown times. */
    fun time(epochMs: Long): String = if (epochMs <= 0) NONE else TIME.format(zoned(epochMs))

    /** Progress fraction in `0..1`, or `null` when the total is unknown. */
    fun fraction(done: Long, total: Long): Float? =
        if (total <= 0) null else (done.toDouble() / total).coerceIn(0.0, 1.0).toFloat()

    /** Whole percent, or `null` when the total is unknown. */
    fun percent(done: Long, total: Long): Int? = fraction(done, total)?.let { (it * 100).toInt() }

    private fun zoned(epochMs: Long) = Instant.ofEpochMilli(epochMs).atZone(ZoneId.systemDefault())
}
