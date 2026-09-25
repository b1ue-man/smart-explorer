package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G5 phase B (own `am instrument` run after the host saw the membership): removing the Room
 * reports the favorites it removed and the sync jobs that still use it (jobs are only reported,
 * like the desktop); a Direct device added by the desktop's code with access request and removal
 * cascade; a second Room created, left and removed.
 */
@RunWith(AndroidJUnit4::class)
class ShareCleanupTaskTest {
    private val desktop: String get() = TaskArgs.get("seDesktopDevice")

    @Test
    fun removingTheRoomReportsFavoritesAndJobs() = coreTest(timeoutMs = 15 * 60_000L) {
        Api.obj("bg.ensureDaemon")
        val room = Share.room { it.text("name") == "Team" }
            ?: throw AssertionError("Raum \"Team\" aus Phase A fehlt: ${Api.obj("share.status").objects("rooms")}")
        val profileId = room.text("profileId")
        val member = Share.awaitMember(profileId, desktop)
        val location = member.text("location")
        assertTrue(Api.obj("loc.toggleFavorite", args("location" to location)).bool("favorite"))
        val local = Fixture.dir(Volumes.primary(), "share", "raum-job")
        val jobName = Fixture.unique("Raum-Job")
        val job = SyncJobs.save("name" to jobName, "source" to local.absolutePath, "target" to location, "enabled" to false)

        val report = Api.obj("share.removeRoom", args("profileId" to profileId))
        TaskReport.file("share-phase-b.txt").writeText("removeRoom=$report\n")
        assertTrue("Favorit nicht gemeldet: $report", report.int("removedFavorites") >= 1)
        assertTrue("Job nicht gemeldet: $report", jobName in report.texts("orphanedJobs"))
        assertTrue(Share.room { it.text("profileId") == profileId } == null)
        assertFalse(Api.obj("loc.isFavorite", args("location" to location)).bool("favorite"))
        SyncJobs.byId(job.text("id"))
        Api.call("sync.delete", args("id" to job.text("id")))
    }

    @Test
    fun directDeviceRequestAndRemovalCascade() = coreTest(timeoutMs = 15 * 60_000L) {
        Api.obj("bg.ensureDaemon")
        val contactId = Api.obj("share.addDirect", args("code" to TaskArgs.get("seDesktopDirectCode"), "name" to "Desktop")).text("contactId")
        var device: JsonObject? = null
        waitFor("Direct-Gerät in share.status", 60_000) {
            device = Api.obj("share.status").objects("devices").firstOrNull { it.text("contactId") == contactId }
            device != null
        }
        assertTrue(device!!.text("location").startsWith("share://direct/"))
        TaskReport.note("share.requestAccess", Api.attempt("share.requestAccess", args("contactId" to contactId, "message" to "Task-Suite")).toString())
        val outgoing = Api.obj("share.status").objects("outgoing").firstOrNull { it.textOrNull("contactId") == contactId }
        TaskReport.note("share.outgoing", outgoing.toString())
        val requestId = outgoing?.text("requestId") ?: "unbekannt"
        TaskReport.note("share.retry", Api.attempt("share.retry", args("requestId" to requestId)).toString())
        TaskReport.note("share.decide", Api.attempt("share.decide", args("requestId" to "unbekannt", "accept" to false)).toString())
        TaskReport.note("share.deleteRequest", Api.attempt("share.deleteRequest", args("requestId" to requestId)).toString())

        val removal = Api.obj("share.removeDevice", args("contactId" to contactId))
        assertTrue(removal.int("removedFavorites") >= 0)
        removal.texts("orphanedJobs")
        assertFalse(Api.obj("share.status").objects("devices").any { it.text("contactId") == contactId })
        val removed = Api.obj("share.status").objects("removedDevices")
        TaskReport.note("share.removedDevices", removed.toString())
        val readmitId = removed.firstOrNull { it.text("deviceId") == desktop }?.text("deviceId") ?: desktop
        Api.call("share.readmit", args("deviceId" to readmitId))
        assertFalse(Api.obj("share.status").objects("removedDevices").any { it.text("deviceId") == desktop })
    }

    @Test
    fun secondRoomCreateLeaveAndRemove() = coreTest {
        Api.obj("bg.ensureDaemon")
        val created = Api.obj("share.createRoom", args("name" to "Task-Raum 2"))
        assertTrue(created.text("code").startsWith("SE-R3-"))
        val profileId = created.text("profileId")
        assertTrue(Share.room { it.text("profileId") == profileId } != null)
        TaskReport.note("share.leaveRoom", Api.attempt("share.leaveRoom", args("profileId" to profileId)).toString())
        Api.obj("share.removeRoom", args("profileId" to profileId))
        assertTrue(Share.room { it.text("profileId") == profileId } == null)
    }
}
