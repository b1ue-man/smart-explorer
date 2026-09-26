package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G5 phase A2 (own `am instrument` run, started in the background while the host script drives
 * `se exec`): the phone is the exec host. It allows the desktop member of Room "Team" to run
 * commands, sees the desktop's command in its history, cancels a running command whose shell
 * started a background child (the whole tree must end), revokes the grant, and gets the host's
 * verdict that the next attempt was refused. Marker files in `<primary>/SmartExplorerTask/exec-host`
 * coordinate both sides (android/test-servers/share-desktop.sh `exec`).
 */
@RunWith(AndroidJUnit4::class)
class ShareExecTaskTest {
    @Test
    fun desktopRunsCommandsOnThePhoneOnlyWhileAllowed() = coreTest(timeoutMs = 20 * 60_000L) {
        Api.obj("bg.ensureDaemon")
        Api.call("share.watch", args("active" to true))
        val desktop = TaskArgs.get("seDesktopDevice")
        val room = Share.room { it.text("name") == "Team" }
            ?: throw AssertionError("Raum \"Team\" aus Phase A fehlt: ${Api.obj("share.status").objects("rooms")}")
        val profileId = room.text("profileId")
        Share.awaitMember(profileId, desktop)
        val status = Api.obj("share.status")
        val provider = status.field("execProvider").obj()
        TaskReport.note("share.execProvider", provider.toString())
        assertTrue("Exec-Anbieter nicht verfügbar: $provider", provider.bool("available"))

        val target = args("kind" to "room", "profileId" to profileId, "deviceId" to desktop)
        Api.call("share.setExec", args("target" to target, "enabled" to true))
        assertTrue(Share.awaitMember(profileId, desktop).field("exec").obj().bool("enabled"))
        val markers = Fixture.dir(Volumes.primary(), "exec-host")
        File(markers, "granted").writeText(status.field("identity").obj().text("deviceId") + "\n")

        // 1. The desktop's command ran (earlier attempts may have failed while presence spread).
        val done = awaitJob("Befehl des Desktops ausgeführt", desktop, active = false) {
            it.text("state") == "exited" && it.textOrNull("exitCode") == "0"
        }
        TaskReport.note("share.execJobs", done.toString())

        // 2. A running command with a background child: cancelling it ends the whole tree.
        val running = awaitJob("laufender Befehl mit Hintergrundkind", desktop, active = true) { true }
        Api.call(
            "share.cancelExecJob",
            args("direction" to "incoming", "execId" to running.text("execId"), "peerDeviceId" to desktop),
        )
        val cancelled = awaitJob("abgebrochener Befehl in der Historie", desktop, active = false) {
            it.text("execId") == running.text("execId")
        }
        assertEquals("cancelled", cancelled.text("state"))
        assertEquals("Prozesse des abgebrochenen Befehls leben weiter", emptyList<String>(), sleepers())

        // 3. Revoked: the host checks that the next attempt is refused and reports back.
        Api.call("share.setExec", args("target" to target, "enabled" to false))
        assertFalse(Share.awaitMember(profileId, desktop).field("exec").obj().bool("enabled"))
        File(markers, "revoked").writeText("1\n")
        val verdict = File(markers, "host-done")
        waitFor("Ergebnis des Desktops nach dem Entzug", 300_000) { verdict.isFile && verdict.readText().isNotBlank() }
        assertEquals("refused", verdict.readText().trim())
        Api.call("share.watch", args("active" to false))
    }

    private suspend fun awaitJob(what: String, peer: String, active: Boolean, match: (JsonObject) -> Boolean): JsonObject {
        var job: JsonObject? = null
        waitFor(what, 300_000) {
            job = Api.obj("share.execJobs").objects(if (active) "active" else "history").firstOrNull {
                it.text("direction") == "incoming" && it.text("peerDeviceId") == peer && match(it)
            }
            job != null
        }
        return job!!
    }

    /** `sleep` processes of the host command (share-desktop.sh) still visible in this app's `/proc`. */
    private fun sleepers(): List<String> =
        File("/proc").listFiles().orEmpty().filter { it.name.all(Char::isDigit) }.mapNotNull { dir ->
            val argv = try {
                File(dir, "cmdline").readText().split('\u0000')
            } catch (e: Exception) {
                return@mapNotNull null
            }
            if (argv.firstOrNull()?.substringAfterLast('/') == "sleep" && argv.getOrNull(1) in setOf("300", "301")) {
                "${dir.name}: ${argv.joinToString(" ")}"
            } else {
                null
            }
        }
}
