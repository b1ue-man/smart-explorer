package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import kotlinx.coroutines.delay
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G5 phase A2 (own `am instrument` run, started in the background while the host script drives
 * `se exec`): the phone is the exec host. It allows the desktop member of Room "Team" to run
 * commands and then observes, in the order share-desktop.sh `exec` sends them:
 * 1. an allowed command that exits 0;
 * 2. a command whose shell leaves a `setsid` child and a double-forked orphan – the phone cancels
 *    it and the whole tree must end (no `sleep 30x` left);
 * 3. the same tree with `--timeout 5` – timed out, nothing left;
 * 4. a running command during which the phone revokes the grant – ended, nothing left;
 * 5. the host's verdict that the next attempt was refused.
 * A background sync job whose "run before" hook sleeps runs in the same app process meanwhile and
 * must stay untouched (the containment may only end the exec job's own processes). Marker files
 * in `<primary>/SmartExplorerTask/exec-host` coordinate both sides.
 */
@RunWith(AndroidJUnit4::class)
class ShareExecTaskTest {
    private val seen = HashSet<String>()

    @Test
    fun desktopRunsCommandsOnThePhoneOnlyWhileAllowed() = coreTest(timeoutMs = 25 * 60_000L) {
        Api.obj("bg.ensureDaemon")
        Api.call("share.watch", args("active" to true))
        val desktop = TaskArgs.get("seDesktopDevice")
        val room = Share.room { it.text("name") == "Team" }
            ?: throw AssertionError("Raum \"Team\" aus Phase A fehlt: ${Api.obj("share.status").objects("rooms")}")
        Share.awaitMember(room.text("profileId"), desktop)
        val status = Api.obj("share.status")
        val provider = status.field("execProvider").obj()
        TaskReport.note("share.execProvider", provider.toString())
        assertTrue("Exec-Anbieter nicht verfügbar: $provider", provider.bool("available"))

        val targetKey = execTarget(desktop).text("targetKey")
        val granted = Api.obj("share.setExec", args("targetKey" to targetKey, "enabled" to true))
        TaskReport.note("share.setExec", granted.toString())
        assertTrue(execTarget(desktop).bool("enabled"))
        val markers = Fixture.dir(Volumes.primary(), "exec-host")
        File(markers, "granted").writeText(status.field("identity").obj().text("deviceId") + "\n")

        // 1. The desktop's command ran (earlier attempts may have failed while presence spread).
        val done = awaitJob("Befehl des Desktops ausgeführt", desktop, active = false) {
            it.text("state") == "exited" && it.textOrNull("exitCode") == "0"
        }
        TaskReport.note("share.execJobs", done.toString())
        val hook = startHookJob()

        // 2. Cancelled on the phone: the shell, its setsid child and the orphan all end.
        val running = awaitJob("laufender Befehl mit setsid-Kind und Waise", desktop, active = true) { true }
        delay(2_000) // let the shell start its children
        Api.call(
            "share.cancelExecJob",
            args("direction" to "incoming", "execId" to running.text("execId"), "peerDeviceId" to desktop),
        )
        val cancelled = awaitJob("abgebrochener Befehl in der Historie", desktop, active = false) {
            it.text("execId") == running.text("execId")
        }
        assertEquals("cancelled", cancelled.text("state"))
        assertEquals("Prozesse des abgebrochenen Befehls leben weiter", emptyList<String>(), sleepers())

        // 3. Timed out on the phone (--timeout 5 from the desktop).
        val timedOut = awaitJob("Befehl mit Zeitlimit beendet", desktop, active = false) { it.text("state") == "timed_out" }
        assertEquals("Prozesse nach dem Zeitlimit leben weiter", emptyList<String>(), sleepers())
        seen += timedOut.text("execId")
        TaskReport.note("share.execJobs", timedOut.toString())

        // 4. Revoked while running: the job ends and leaves nothing behind.
        val last = awaitJob("laufender Befehl vor dem Entzug", desktop, active = true) { true }
        delay(2_000)
        Api.obj("share.setExec", args("targetKey" to targetKey, "enabled" to false))
        assertFalse(execTarget(desktop).bool("enabled"))
        val revoked = awaitJob("Befehl nach dem Entzug beendet", desktop, active = false) { it.text("execId") == last.text("execId") }
        assertTrue("Zustand nach Entzug: ${revoked.text("state")}", revoked.text("state") in setOf("revoked", "cancelled"))
        assertEquals("Prozesse nach dem Entzug leben weiter", emptyList<String>(), sleepers())
        File(markers, "revoked").writeText("1\n")

        // 5. The desktop's next attempt was refused.
        val verdict = File(markers, "host-done")
        waitFor("Ergebnis des Desktops nach dem Entzug", 300_000) { verdict.isFile && verdict.readText().isNotBlank() }
        assertEquals("refused", verdict.readText().trim())

        // The hook's own shell was neither killed nor reaped by the exec containment.
        waitFor("Hintergrund-Job mit Vorher-Befehl fertig", 300_000) { hook.second.isFile }
        Api.call("sync.delete", args("id" to hook.first))
        Api.call("share.watch", args("active" to false))
    }

    private suspend fun execTarget(deviceId: String): JsonObject =
        Api.obj("share.status").objects("execTargets").firstOrNull { it.text("relation") == "room" && it.text("deviceId") == deviceId }
            ?: throw AssertionError("Kein Exec-Ziel für $deviceId: ${Api.obj("share.status")["execTargets"]}")

    /** Waits for a matching job; an active one must be new (the host sends one command at a time). */
    private suspend fun awaitJob(what: String, peer: String, active: Boolean, match: (JsonObject) -> Boolean): JsonObject {
        var job: JsonObject? = null
        waitFor(what, 300_000) {
            job = Api.obj("share.execJobs").objects(if (active) "active" else "history").firstOrNull {
                it.text("direction") == "incoming" && it.text("peerDeviceId") == peer && match(it) &&
                    (!active || it.text("execId") !in seen)
            }
            job != null
        }
        if (active) seen += job!!.text("execId")
        return job!!
    }

    /** A due interval job with a sleeping "run before" hook, run by the daemon (bg.catchUp). */
    private suspend fun startHookJob(): Pair<String, File> {
        Api.call("bg.setSyncEnabled", args("enabled" to true))
        Api.call("bg.resume")
        val source = Fixture.dir(Volumes.primary(), "exec-host-hook", "quelle")
        val target = Fixture.dir(Volumes.primary(), "exec-host-hook", "ziel")
        Fixture.write(File(source, "nach-dem-hook.txt"), "hook")
        val job = SyncJobs.save(
            "name" to Fixture.unique("Exec-Hook-Job"),
            "source" to source.absolutePath,
            "target" to target.absolutePath,
            "direction" to "a2b",
            "trigger" to "interval",
            "intervalMin" to 5,
            "catchUp" to true,
            "runBefore" to "sleep 60",
            "enabled" to true,
        )
        Api.start("bg.catchUp")
        return job.text("id") to File(target, "nach-dem-hook.txt")
    }

    /** `sleep 30x` processes of the host commands (share-desktop.sh) still visible in `/proc`. */
    private fun sleepers(): List<String> =
        File("/proc").listFiles().orEmpty().filter { it.name.all(Char::isDigit) }.mapNotNull { dir ->
            val argv = try {
                File(dir, "cmdline").readText().split('\u0000')
            } catch (e: Exception) {
                return@mapNotNull null
            }
            val program = argv.firstOrNull()?.substringAfterLast('/')
            if (program == "sleep" && argv.getOrNull(1)?.matches(Regex("30[0-9]")) == true) {
                "${dir.name}: ${argv.joinToString(" ")}"
            } else {
                null
            }
        }
}
