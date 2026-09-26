package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.service.execNotificationTitle
import app.smartexplorer.android.service.execProgramText
import app.smartexplorer.android.service.freeExecNotificationId
import app.smartexplorer.android.service.runningOnThisPhone
import app.smartexplorer.android.system.Notifications
import app.smartexplorer.android.ui.share.execJobTitle
import app.smartexplorer.android.ui.share.execRelationText
import app.smartexplorer.android.ui.share.execStateLabel
import app.smartexplorer.android.ui.share.recentIncoming
import app.smartexplorer.android.ui.share.runningIncoming
import kotlinx.serialization.KSerializer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The exec-host answers of api.md §5 (`execProvider`/`execTargets` of `share.status`,
 * `share.execJobs`) decoded with the app's DTOs and `Core.json`, plus the texts and selections the
 * Share page and the notification build from them.
 */
class ShareExecApiTest {
    private fun <T> decode(serializer: KSerializer<T>, text: String): T = Core.json.decodeFromString(serializer, text)

    private val jobs = decode(
        ExecJobs.serializer(),
        """{"active":[
             {"direction":"incoming","execId":"aa","peerDeviceId":"d1","peerName":"Laptop","program":"<shell>",
              "state":"running","startedAt":1727262000,"finishedAt":null,"exitCode":null,"message":null},
             {"direction":"outgoing","execId":"bb","peerDeviceId":"d2","peerName":"NAS","program":"ls",
              "state":"queued_local","startedAt":null}],
           "history":[
             {"direction":"incoming","execId":"cc","peerDeviceId":"d1","peerName":"Laptop","program":"<shell>",
              "state":"timed_out","startedAt":1727262000,"finishedAt":1727262005,"exitCode":137,"message":"Zeitlimit"},
             {"direction":"incoming","execId":"dd","peerDeviceId":"d1","peerName":"","program":"uname",
              "state":"exited","startedAt":1727262100,"finishedAt":1727262101,"exitCode":0,"message":null},
             {"direction":"outgoing","execId":"ee","peerDeviceId":"d2","peerName":"NAS","program":"ls",
              "state":"cancelled","startedAt":1727262200,"finishedAt":1727262300}]}""",
    )

    @Test
    fun statusCarriesTheExecProviderAndTargets() {
        val status = decode(
            ShareStatus.serializer(),
            """{"running":true,"connected":true,"identity":{},"discovery":{},
               "execProvider":{"available":true,"provider":"android-subreaper","detail":"Zwischenprozess je Befehl"},
               "execTargets":[
                 {"targetKey":"direct/d1/fp1","relation":"direct","roomId":null,"roomName":null,"deviceId":"d1",
                  "name":"Laptop","fingerprint":"fp1","enabled":true,"baseAuthorized":true,"policyRevision":7},
                 {"targetKey":"room/wire/d2/fp2","relation":"room","roomId":"wire","roomName":"Team","deviceId":"d2",
                  "name":"Desktop","fingerprint":"fp2","enabled":false,"baseAuthorized":false,"policyRevision":0}]}""",
        )
        assertTrue(status.execProvider.available)
        assertEquals("android-subreaper", status.execProvider.provider)
        val (direct, member) = status.execTargets
        assertEquals("direct/d1/fp1", direct.targetKey)
        assertTrue(direct.enabled)
        assertEquals(7L, direct.policyRevision)
        assertNull(direct.roomName)
        assertEquals("Direkt-Gerät", execRelationText(direct))
        assertEquals(EXEC_RELATION_ROOM, member.relation)
        assertFalse(member.baseAuthorized)
        assertEquals("Raum „Team“", execRelationText(member))

        // An answer without the exec fields (older core) keeps the section safe: nothing allowed.
        val older = decode(ShareStatus.serializer(), """{"running":false,"identity":{},"discovery":{}}""")
        assertFalse(older.execProvider.available)
        assertTrue(older.execTargets.isEmpty())
    }

    @Test
    fun jobsDecodeWithEveryField() {
        val running = jobs.active.first()
        assertEquals(EXEC_INCOMING, running.direction)
        assertEquals("running", running.state)
        assertEquals(1727262000L, running.startedAt)
        assertNull(running.exitCode)
        val timedOut = jobs.history.first()
        assertEquals("timed_out", timedOut.state)
        assertEquals(137, timedOut.exitCode)
        assertEquals("Zeitlimit", timedOut.message)
        assertEquals(1727262005L, timedOut.finishedAt)
        assertEquals(0, decode(ExecJobs.serializer(), "{}").active.size)
    }

    @Test
    fun theSectionAndTheNotificationShowOnlyCommandsOnThisPhone() {
        assertEquals(listOf("aa"), runningIncoming(jobs).map { it.execId })
        assertEquals(listOf("aa"), runningOnThisPhone(jobs).map { it.execId })
        assertTrue(runningIncoming(null).isEmpty())
        // Newest first, outgoing commands left out, at most the limit.
        assertEquals(listOf("dd", "cc"), recentIncoming(jobs, 10).map { it.execId })
        assertEquals(listOf("dd"), recentIncoming(jobs, 1).map { it.execId })

        assertEquals("Laptop führt einen Befehl aus", execNotificationTitle(jobs.active.first()))
        assertEquals("Ein Gerät führt einen Befehl aus", execNotificationTitle(jobs.history[1]))
        assertEquals("Shell-Befehl", execProgramText(jobs.active.first()))
        assertEquals("uname", execProgramText(jobs.history[1]))
        assertEquals("Laptop: Shell-Befehl", execJobTitle(jobs.active.first()))
        assertEquals("Gerät: uname", execJobTitle(jobs.history[1]))
    }

    @Test
    fun statesHaveGermanLabels() {
        assertEquals("Läuft", execStateLabel("running"))
        assertEquals("Gestoppt", execStateLabel("cancelled"))
        assertEquals("Zeitlimit erreicht", execStateLabel("timed_out"))
        assertEquals("Erlaubnis entzogen", execStateLabel("revoked"))
        assertEquals("Beendet", execStateLabel("exited"))
        assertEquals("neu_im_kern", execStateLabel("neu_im_kern"))
    }

    @Test
    fun everyRunningCommandGetsItsOwnNotificationId() {
        val first = Notifications.ID_EXEC_FIRST
        assertEquals(first, freeExecNotificationId(emptyList()))
        assertEquals(first + 1, freeExecNotificationId(listOf(first, first + 2)))
        val all = (first until first + Notifications.EXEC_ID_COUNT).toList()
        assertNull(freeExecNotificationId(all))
    }
}
