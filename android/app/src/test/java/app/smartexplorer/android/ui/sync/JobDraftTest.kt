package app.smartexplorer.android.ui.sync

import app.smartexplorer.android.api.SyncCalendar
import app.smartexplorer.android.api.SyncChoice
import app.smartexplorer.android.api.SyncJob
import app.smartexplorer.android.api.SyncOptions
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Job editor draft (spec F15): calendar weekday bitmask (bit0 = Mo … bit6 = So), times and ranges. */
class JobDraftTest {
    private val options = SyncOptions(
        calendarKinds = listOf(SyncChoice("daily", "Täglich"), SyncChoice("weekly", "Wöchentlich"), SyncChoice("monthly", "Monatlich")),
    )

    private fun draft(job: SyncJob = SyncJob(name = "Fotos", source = "/s/a", target = "/s/b")) = JobDraft(job, options)

    @Test
    fun weeklyCalendarKeepsTheDesktopBitmask() {
        val d = draft()
        d.trigger = SyncJob.TRIGGER_CALENDAR
        d.calendarKind = "weekly"
        d.weekdays = 0b0000101 // Monday and Wednesday
        d.calendarTime = "07:30"
        val (job, problems) = d.build()
        assertTrue(problems.toString(), problems.isEmpty())
        assertEquals(SyncCalendar(kind = "weekly", minuteOfDay = 450, weekday = 5, monthday = 0), job.calendar)
        d.weekdays = 0
        assertEquals("Mindestens einen Wochentag wählen", d.build().second["calendar"])
    }

    @Test
    fun existingBitmaskIsReadBackAndMasked() {
        val job = SyncJob(id = "j1", source = "/a", target = "/b", trigger = SyncJob.TRIGGER_CALENDAR, calendar = SyncCalendar("weekly", 60, 0x1ff, 1))
        val d = draft(job)
        assertEquals(0x7f, d.weekdays)
        assertEquals("01:00", d.calendarTime)
        assertFalse(d.isNew)
    }

    @Test
    fun monthlyDayAndNumbersAreRangeChecked() {
        val d = draft()
        d.trigger = SyncJob.TRIGGER_CALENDAR
        d.calendarKind = "monthly"
        d.monthday = "31"
        assertEquals(31, d.build().first.calendar!!.monthday)
        d.monthday = "32"
        assertEquals("Ganze Zahl von 1 bis 31 eingeben", d.build().second["calendar"])
        d.trigger = SyncJob.TRIGGER_INTERVAL
        d.intervalMin = "0"
        assertEquals("Ganze Zahl von 1 bis 10080 eingeben", d.build().second["intervalMin"])
        d.intervalMin = "15"
        d.maxDeletePct = "101"
        val (job, problems) = d.build()
        assertEquals(15, job.intervalMin)
        assertEquals(setOf("maxDeletePct"), problems.keys)
    }

    @Test
    fun activeWindowNeedsBothTimes() {
        val d = draft()
        d.activeFrom = "08:00"
        d.activeTo = ""
        assertTrue("activeFromMin" in d.build().second)
        d.activeTo = "18:30"
        val job = d.build().first
        assertEquals(480, job.activeFromMin)
        assertEquals(1110, job.activeToMin)
        val blank = draft(SyncJob(source = "", target = "")).build().second
        assertEquals(setOf("source", "target"), blank.keys)
    }

    @Test
    fun timeTextConversion() {
        assertEquals(420, JobDraft.textToMinutes("7"))
        assertEquals(1439, JobDraft.textToMinutes("23:59"))
        assertNull(JobDraft.textToMinutes("24:00"))
        assertNull(JobDraft.textToMinutes("x"))
        assertEquals("23:59", JobDraft.minutesToText(1439))
        assertEquals("00:00", JobDraft.minutesToText(24 * 60))
        assertTrue(JobDraft.isWeekly("weekly"))
        assertTrue(JobDraft.isMonthly("Monthly"))
    }
}
