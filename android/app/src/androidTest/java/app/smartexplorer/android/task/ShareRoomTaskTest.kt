package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.Entry
import java.io.File
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G5 phase A (own `am instrument` run, arguments from the host script): the phone joins the Room
 * the desktop CLI created on the runner's Share server, sees the desktop as a member, lists its
 * Room export and downloads the test file over `share://room/…` (content compared by SHA-256).
 * Also exports a phone folder into the Room, runs the discoverable-offer lifecycle and a
 * `share.exec` attempt. The host then checks the desktop side of the membership.
 */
@RunWith(AndroidJUnit4::class)
class ShareRoomTaskTest {
    @Test
    fun joinTheDesktopRoomAndDownloadItsFile() = coreTest(timeoutMs = 20 * 60_000L) {
        val server = TaskArgs.get("seShareServer")
        val desktop = TaskArgs.get("seDesktopDevice")
        val folder = TaskArgs.get("seRoomFolder")
        val fileName = TaskArgs.get("seRoomFile")
        Share.goOnline(server)
        val online = Api.obj("share.status")
        val identity = online.field("identity").obj()
        // Evidence for the Android interface heuristic (net/os/android/interfaces.rs, B1a open point).
        TaskReport.note("share.lanPresence", online.text("lanPresence"))
        TaskReport.file("share-phone-device.txt").writeText(identity.text("deviceId") + "\n")

        val profileId = Api.obj("share.joinRoom", args("code" to TaskArgs.get("seRoomCode"), "name" to "Team")).text("profileId")
        assertNotNull(Api.obj("share.roomCode", args("profileId" to profileId)).textOrNull("code"))
        val member = Share.awaitMember(profileId, desktop)
        val location = member.text("location")
        assertTrue(location.startsWith("share://room/"))

        var docs: Entry? = null
        waitFor("Raumfreigabe \"$folder\" des Desktops", 240_000) {
            docs = try {
                Api.listing(location).entries.firstOrNull { it.name == folder }
            } catch (e: CoreException) {
                TaskReport.note("share", "Liste $location: ${e.kind} ${e.message}")
                null
            }
            docs != null
        }
        val remoteFile = Api.child(docs!!.location, fileName)
        val target = Fixture.dir(Volumes.primary(), "share", "empfangen")
        Api.copy(listOf(remoteFile.location), target.absolutePath)
        assertEquals(TaskArgs.get("seRoomFileSha256"), Fixture.sha256(File(target, fileName)))

        val export = Fixture.dir(Volumes.primary(), "share", "telefon-freigabe")
        Fixture.write(File(export, "vom-telefon.txt"), "Telefon")
        Api.call("share.addExport", args("scope" to profileId, "path" to export.absolutePath, "label" to "PhoneDocs"))
        val exports = Api.obj("share.status").field("exports").obj().field("rooms").obj()[profileId]
        assertTrue("Raumfreigabe fehlt: $exports", exports.toString().contains("PhoneDocs"))
        Api.call("share.removeExport", args("scope" to profileId, "path" to export.absolutePath))

        Api.call("share.discoverable", args("target" to "direct", "alias" to "SE Task", "pin" to "4826", "minutes" to 5))
        var offer: JsonObject? = null
        waitFor("eigenes Discovery-Angebot", 60_000) {
            offer = Api.obj("share.status").field("discovery").obj()["offer"] as? JsonObject
            offer != null
        }
        Api.call("share.stopDiscoverable", args("offerId" to offer!!.text("offerId")))
        waitFor("Discovery-Angebot beendet", 60_000) { Api.obj("share.status").field("discovery").obj()["offer"] !is JsonObject }
        Api.call("share.discover")
        // PIN pairing needs a second device with a running offer; the desktop CLI has none
        // (explicit exception): only the contract's answers to unknown ids are exercised.
        TaskReport.note("share.connect", Api.attempt("share.connect", args("discoveryId" to "unbekannt", "pin" to "4826")).toString())
        TaskReport.note("share.cancelConnect", Api.attempt("share.cancelConnect", args("exchangeId" to "unbekannt")).toString())

        val exec = Api.await(Api.start("share.exec", args("location" to location, "command" to "echo hallo", "shell" to false, "timeoutSecs" to 15)), 120_000)
        TaskReport.note("share.exec", "${exec.state}: ${exec.message} ${exec.result}")
        Api.call("share.watch", args("active" to false))
        TaskReport.file("share-phase-a.txt").writeText("profileId=$profileId\nmember=$location\n")
    }
}

/** Share helpers shared by both G5 phases. */
object Share {
    suspend fun goOnline(server: String) {
        Api.obj("bg.ensureDaemon")
        Api.call("share.setServer", args("server" to server))
        Api.call("share.setName", args("name" to "Android-Task"))
        Api.call("share.setOnline", args("online" to true))
        Api.call("share.watch", args("active" to true))
        waitFor("Share-Dienst mit dem Server verbunden", 240_000) {
            val status = Api.obj("share.status")
            status.bool("running") && status.bool("connected")
        }
        val configured = Api.obj("share.status").text("server")
        assertTrue("Share-Server $configured statt $server", configured.contains(server))
    }

    suspend fun room(predicate: (JsonObject) -> Boolean): JsonObject? =
        Api.obj("share.status").objects("rooms").firstOrNull(predicate)

    suspend fun awaitMember(profileId: String, deviceId: String): JsonObject {
        var member: JsonObject? = null
        waitFor("Gerät $deviceId als Mitglied im Raum $profileId", 240_000) {
            member = room { it.text("profileId") == profileId }?.objects("members")?.firstOrNull { it.text("deviceId") == deviceId }
            member != null
        }
        return member!!
    }
}
