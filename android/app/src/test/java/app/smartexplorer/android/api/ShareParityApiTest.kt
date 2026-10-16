package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.ui.share.execAuditIsRefusal
import app.smartexplorer.android.ui.share.execAuditTime
import app.smartexplorer.android.ui.share.execProgramsText
import app.smartexplorer.android.ui.share.execRunAsText
import app.smartexplorer.android.ui.share.programInputError
import kotlinx.serialization.KSerializer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * GUI parity of the Share and Exec settings (2026-10-16): the new answer fields of `share.status`
 * and `share.execLog` decoded with the app's DTOs, and the texts and checks the Share page builds
 * from them (account, program list, audit entries).
 */
class ShareParityApiTest {
    private fun <T> decode(serializer: KSerializer<T>, text: String): T = Core.json.decodeFromString(serializer, text)

    @Test
    fun lueckenTaskStatusCarriesAccountProgramsAndDeviceSettings() {
        val status = decode(
            ShareStatus.serializer(),
            """{"running":true,"identity":{},"discovery":{},"lanPresenceEnabled":false,
               "execProvider":{"available":true,"provider":"android-subreaper","detail":"","userLabel":"u0_a123","runAsSupported":false},
               "execTargets":[{"targetKey":"direct/d1/fp1","enabled":true,"runAs":"standard","programs":["uptime","/system/bin/df"]}],
               "devices":[{"contactId":"c1","name":"Laptop","autoConnect":false,"autoOpen":true}]}""",
        )
        assertFalse(status.lanPresenceEnabled)
        assertEquals("u0_a123", status.execProvider.userLabel)
        val target = status.execTargets.single()
        assertEquals(listOf("uptime", "/system/bin/df"), target.programs)
        assertEquals("Standardnutzer (u0_a123, Konto der App)", execRunAsText(target, status.execProvider))
        assertEquals("nur uptime, /system/bin/df", execProgramsText(target.programs))
        val device = status.devices.single()
        assertFalse(device.autoConnect)
        assertTrue(device.autoOpen)

        // An answer of an older core keeps the safe defaults: full shell, standard account, LAN on.
        val older = decode(
            ShareStatus.serializer(),
            """{"identity":{},"discovery":{},"execTargets":[{"targetKey":"k"}],"devices":[{"contactId":"c"}]}""",
        )
        assertTrue(older.lanPresenceEnabled)
        assertEquals(EXEC_RUN_AS_STANDARD, older.execTargets.single().runAs)
        assertEquals("volle Shell", execProgramsText(older.execTargets.single().programs))
        assertTrue(older.devices.single().autoConnect)
        assertEquals("Konto deploy (auf Android nicht möglich)", execRunAsText(ExecTarget("k", runAs = "deploy"), ExecProviderInfo()))
    }

    @Test
    fun lueckenTaskProgramEntriesFollowTheCoreRules() {
        val draft = listOf("uptime")
        assertNull(programInputError(draft, " /system/bin/df "))
        assertNull(programInputError(draft, "id"))
        assertNotNull(programInputError(draft, "uptime"))
        assertNotNull(programInputError(draft, "   "))
        assertNotNull(programInputError(draft, "bin/tool"))
        assertNotNull(programInputError(draft, "./tool"))
        assertNotNull(programInputError(draft, "a\u0000b"))
    }

    @Test
    fun lueckenTaskAuditLogDecodesAndMarksRefusals() {
        val log = decode(
            ExecAuditLog.serializer(),
            """{"max":1000,"entries":[
                 {"at":"2026-10-16T10:00:03Z","event":"finished","title":"Beendet","device":"Laptop (d1)","deviceId":"d1","detail":"Exit: 0"},
                 {"at":"","event":"unreadable","title":"Unlesbare Zeile","device":"","deviceId":"","detail":"x"},
                 {"at":"2026-10-16T10:00:01Z","event":"auth_rejected","title":"Anmeldung abgelehnt","device":"d9","deviceId":"d9","detail":"Grund: x"}]}""",
        )
        assertEquals(1000, log.max)
        assertEquals(3, log.entries.size)
        assertFalse(execAuditIsRefusal(log.entries[0]))
        assertTrue(execAuditIsRefusal(log.entries[1]))
        assertTrue(execAuditIsRefusal(log.entries[2]))
        assertEquals("–", execAuditTime(""))
        assertEquals("kein Datum", execAuditTime("kein Datum"))
        assertTrue(execAuditTime("2026-10-16T10:00:03Z").matches(Regex("""\d\d\.\d\d\.2026 \d\d:\d\d:\d\d""")))
    }
}
