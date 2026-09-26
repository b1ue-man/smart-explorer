package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.api.Listing
import java.io.File
import kotlinx.coroutines.delay
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G4 remote flows against the servers on the runner (10.0.2.2): SFTP files, uploads with the
 * protected trash omission, downloads, remote edits with conflict and forced overwrite over the
 * Remote-Agent, atomic replace on plain SFTP (posix-rename), host-key reset, FTP basics (upload,
 * download, replace), SMB (Samba: upload, download, atomic replace, copy, delete), WebDAV validation (live WebDAV needs a publicly trusted HTTPS certificate,
 * explicit exception) and the cleanup cascade of `conn.delete`.
 */
@RunWith(AndroidJUnit4::class)
class RemoteTaskTest {
    @Test
    fun sftpUploadDownloadDeleteAndMaterialize() = coreTest {
        assertTrue(Api.obj("conn.test", args("input" to Servers.sftpInput())).text("message").isNotBlank())
        Api.failure("conn.test", args("input" to Servers.sftpInput(password = "falsches-passwort")), "auth")
        val connection = Servers.sftp()
        assertTrue(Api.objects("conn.list").any { it.text("id") == connection.text("id") })
        val root = Api.listing(connection.text("location"))
        assertEquals("sftp", root.backend)
        assertFalse(root.readOnly)

        val remote = Servers.freshDir(connection, "dateien")
        val local = Fixture.dir(Volumes.primary(), "remote", "upload")
        Fixture.write(File(local, "paket/behalten.txt"), "behalten")
        Fixture.bytes(File(local, "paket/daten.bin"), 300_000, 5)
        Fixture.write(File(local, "paket/.SmartExplorer-Papierkorb/geheim.txt"), "nicht hochladen")
        val upload = Api.copy(listOf(File(local, "paket").absolutePath), remote).resultObj()
        assertTrue("Auslassung des Papierkorb-Ordners nicht gemeldet: $upload", upload.long("omitted") >= 1)
        val paket = Api.child(remote, "paket").location
        assertEquals(setOf("behalten.txt", "daten.bin"), Api.names(paket))

        Api.failure(
            "fs.transfer",
            args("sources" to listOf(paket), "targetDir" to local.absolutePath, "mode" to "move", "conflict" to "keepBoth"),
            "unsupported",
        )
        val downloadDir = Fixture.dir(Volumes.sdCard(), "remote", "download")
        Api.copy(listOf(Api.child(paket, "daten.bin").location), downloadDir.absolutePath)
        assertEquals(Fixture.sha256(File(local, "paket/daten.bin")), Fixture.sha256(File(downloadDir, "daten.bin")))
        // A second upload of the same name gets a number, never replaces (desktop rule).
        Api.copy(listOf(File(local, "paket/behalten.txt").absolutePath), paket)
        assertTrue("behalten (2).txt" in Api.names(paket))

        val materialized = Api.runTask("fs.materialize", args("locations" to listOf(Api.child(paket, "behalten.txt").location))).resultObj()
        val copy = File(materialized.texts("paths").single())
        assertTrue(copy.path.startsWith(appContext.cacheDir.absolutePath))
        assertEquals("behalten", copy.readText())

        val doomed = Api.child(paket, "behalten (2).txt").location
        Api.failure("fs.delete", args("locations" to listOf(doomed), "permanent" to false), "unsupported")
        Api.runTask("fs.delete", args("locations" to listOf(doomed), "permanent" to true))
        assertFalse("behalten (2).txt" in Api.names(paket))

        Api.call("conn.forgetHostKey", args("id" to connection.text("id")))
        assertTrue(Api.obj("conn.test", args("input" to Servers.sftpInput())).text("message").isNotBlank())
    }

    /** Remote-Agent (desktop option): it replaces files, so a remote edit can conflict and be forced. */
    @Test
    fun remoteEditsConflictForceCopyAndDiscard() = coreTest {
        assertTrue(Api.obj("conn.test", args("input" to Servers.agentSftpInput())).text("message").isNotBlank())
        val connection = Servers.agentSftp()
        assertTrue("Remote-Agent nicht gespeichert: $connection", connection.bool("useAgent"))
        val remote = Servers.freshDir(connection, "edits")
        val local = Fixture.dir(Volumes.primary(), "remote", "edits")
        Api.copy(listOf(Fixture.write(File(local, "text.txt"), "Version 0").absolutePath), remote)
        val location = Api.child(remote, "text.txt").location

        val first = Api.runTask("fs.fetch", args("location" to location)).resultObj()
        delay(2_500) // remote mtimes have one-second resolution
        val second = Api.runTask("fs.fetch", args("location" to location)).resultObj()
        assertTrue(first.text("editId") != second.text("editId"))

        File(second.text("localPath")).writeText("Version B – zuerst hochgeladen")
        val edits = Api.objects("fs.edits")
        assertTrue(edits.any { it.text("editId") == second.text("editId") && it.bool("modified") })
        Api.runTask("fs.uploadEdit", args("editId" to second.text("editId"), "mode" to "overwrite"))

        File(first.text("localPath")).writeText("Version A – später gespeichert, länger")
        val conflict = Api.await(Api.start("fs.uploadEdit", args("editId" to first.text("editId"), "mode" to "overwrite")))
        assertEquals("failed", conflict.state)
        assertTrue("Konflikt nicht gemeldet: ${conflict.result}", conflict.resultObj().bool("conflict"))
        Api.runTask("fs.uploadEdit", args("editId" to first.text("editId"), "mode" to "overwrite", "force" to true))
        val check = Api.runTask("fs.fetch", args("location" to location)).resultObj()
        assertEquals("Version A – später gespeichert, länger", File(check.text("localPath")).readText())

        Api.runTask("fs.uploadEdit", args("editId" to second.text("editId"), "mode" to "copy"))
        assertTrue("text (2).txt" in Api.names(remote))

        for (edit in listOf(first, second, check)) Api.call("fs.discardEdit", args("editId" to edit.text("editId")))
        val remaining = Api.objects("fs.edits").map { it.text("editId") }
        assertFalse(remaining.any { it in setOf(first.text("editId"), second.text("editId"), check.text("editId")) })
        assertFalse(File(first.text("localPath")).exists())
    }

    /** Plain SFTP replaces an existing file atomically (posix-rename@openssh.com); a copy still works. */
    @Test
    fun plainSftpEditsReplaceAtomicallyAndUploadACopy() = coreTest {
        val connection = Servers.sftp()
        val remote = Servers.freshDir(connection, "edits-sftp")
        val local = Fixture.dir(Volumes.primary(), "remote", "edits-sftp")
        Api.copy(listOf(Fixture.write(File(local, "notiz.txt"), "Version 0").absolutePath), remote)
        val location = Api.child(remote, "notiz.txt").location

        val edit = Api.runTask("fs.fetch", args("location" to location)).resultObj()
        File(edit.text("localPath")).writeText("Version 1")
        Api.runTask("fs.uploadEdit", args("editId" to edit.text("editId"), "mode" to "overwrite"))
        assertEquals(setOf("notiz.txt"), Api.names(remote))
        val replaced = Api.runTask("fs.fetch", args("location" to location)).resultObj()
        assertEquals("Version 1", File(replaced.text("localPath")).readText())

        File(edit.text("localPath")).writeText("Version 2")
        Api.runTask("fs.uploadEdit", args("editId" to edit.text("editId"), "mode" to "copy"))
        assertEquals(setOf("notiz.txt", "notiz (2).txt"), Api.names(remote))
        for (item in listOf(edit, replaced)) Api.call("fs.discardEdit", args("editId" to item.text("editId")))
    }

    @Test
    fun ftpBasics() = coreTest {
        assertTrue(Api.obj("conn.test", args("input" to Servers.ftpInput())).text("message").isNotBlank())
        val connection = Servers.ftp()
        val listing: Listing = Api.listing(connection.text("location"))
        assertEquals("ftp", listing.backend)
        val remote = Servers.freshDir(connection, "ftp")
        val local = Fixture.dir(Volumes.primary(), "remote", "ftp")
        val file = Fixture.bytes(File(local, "ftp.bin"), 150_000, 11)
        // FTP has no exclusive create: the private stage is checked absent right before its STOR.
        Api.copy(listOf(file.absolutePath), remote)
        assertEquals(setOf("ftp.bin"), Api.names(remote))
        val downloaded = Fixture.dir(Volumes.primary(), "remote", "ftp-down")
        Api.copy(listOf(Api.child(remote, "ftp.bin").location), downloaded.absolutePath)
        assertEquals(Fixture.sha256(file), Fixture.sha256(File(downloaded, "ftp.bin")))
        // Replacing an existing file is one RNFR/RNTO on the server.
        Api.copy(listOf(Fixture.write(File(local, "text.txt"), "Version 0").absolutePath), remote)
        val edit = Api.runTask("fs.fetch", args("location" to Api.child(remote, "text.txt").location)).resultObj()
        File(edit.text("localPath")).writeText("Version 1")
        Api.runTask("fs.uploadEdit", args("editId" to edit.text("editId"), "mode" to "overwrite"))
        assertEquals(setOf("ftp.bin", "text.txt"), Api.names(remote))
        val check = Api.runTask("fs.fetch", args("location" to Api.child(remote, "text.txt").location)).resultObj()
        assertEquals("Version 1", File(check.text("localPath")).readText())
        for (item in listOf(edit, check)) Api.call("fs.discardEdit", args("editId" to item.text("editId")))
        // Downloads work: a file the server side placed in the user's home.
        val fixture = TaskArgs.get("seFtpFixture")
        val back = Fixture.dir(Volumes.primary(), "remote", "ftp-back")
        Api.copy(listOf(Api.child(connection.text("location"), fixture).location), back.absolutePath)
        assertEquals(TaskArgs.get("seFtpFixtureSha256"), Fixture.sha256(File(back, fixture)))
        Api.runTask("fs.delete", args("locations" to listOf(remote), "permanent" to true))
        assertFalse(remote.substringAfterLast('/') in Api.names(connection.text("location")))
    }

    @Test
    fun smbUploadDownloadReplaceAndDelete() = coreTest {
        assertTrue(Api.obj("conn.test", args("input" to Servers.smbInput())).text("message").isNotBlank())
        Api.failure("conn.test", args("input" to Servers.smbInput(password = "falsches-passwort")), "auth")
        // A wrong share name is named as such, not reported as a missing folder deeper down.
        val wrongShare = Api.failure("conn.test", args("input" to Servers.smbInput().withFields("root" to "/gibt-es-nicht")), "not_found")
        assertTrue("Freigabe nicht benannt: ${wrongShare.message}", wrongShare.message.orEmpty().contains("gibt-es-nicht"))
        val connection = Servers.smb()
        val listing: Listing = Api.listing(connection.text("location"))
        assertEquals("smb", listing.backend)
        assertFalse(listing.readOnly)
        val remote = Servers.freshDir(connection, "smb")
        val local = Fixture.dir(Volumes.primary(), "remote", "smb")
        val file = Fixture.bytes(File(local, "smb.bin"), 300_000, 13)
        Api.copy(listOf(file.absolutePath), remote)
        assertEquals(setOf("smb.bin"), Api.names(remote))
        val downloaded = Fixture.dir(Volumes.sdCard(), "remote", "smb-down")
        Api.copy(listOf(Api.child(remote, "smb.bin").location), downloaded.absolutePath)
        assertEquals(Fixture.sha256(file), Fixture.sha256(File(downloaded, "smb.bin")))
        // A second upload of the same name gets a number, never replaces (desktop rule).
        Api.copy(listOf(file.absolutePath), remote)
        assertEquals(setOf("smb.bin", "smb (2).bin"), Api.names(remote))

        // Replacing is one rename with ReplaceIfExists on the server; a copy upload still works.
        Api.copy(listOf(Fixture.write(File(local, "notiz.txt"), "Version 0").absolutePath), remote)
        val location = Api.child(remote, "notiz.txt").location
        val edit = Api.runTask("fs.fetch", args("location" to location)).resultObj()
        File(edit.text("localPath")).writeText("Version 1")
        Api.runTask("fs.uploadEdit", args("editId" to edit.text("editId"), "mode" to "overwrite"))
        val check = Api.runTask("fs.fetch", args("location" to location)).resultObj()
        assertEquals("Version 1", File(check.text("localPath")).readText())
        File(edit.text("localPath")).writeText("Version 2")
        Api.runTask("fs.uploadEdit", args("editId" to edit.text("editId"), "mode" to "copy"))
        assertEquals(setOf("smb.bin", "smb (2).bin", "notiz.txt", "notiz (2).txt"), Api.names(remote))
        for (item in listOf(edit, check)) Api.call("fs.discardEdit", args("editId" to item.text("editId")))

        // Downloads of a file the server side placed in the share.
        val fixture = TaskArgs.get("seSmbFixture")
        val back = Fixture.dir(Volumes.primary(), "remote", "smb-back")
        Api.copy(listOf(Api.child(connection.text("location"), fixture).location), back.absolutePath)
        assertEquals(TaskArgs.get("seSmbFixtureSha256"), Fixture.sha256(File(back, fixture)))

        Api.call("fs.rename", args("location" to Api.child(remote, "smb (2).bin").location, "newName" to "umbenannt.bin"))
        assertTrue("umbenannt.bin" in Api.names(remote))
        Api.runTask("fs.delete", args("locations" to listOf(remote), "permanent" to true))
        assertFalse(remote.substringAfterLast('/') in Api.names(connection.text("location")))
    }

    @Test
    fun webdavValidationOnly() = coreTest {
        val input = args(
            "label" to "Task WebDAV",
            "protocol" to "webdav",
            "host" to Servers.host,
            "port" to 9,
            "user" to "dav",
            "root" to "/",
            "auth" to "password",
            "useAgent" to false,
            "https" to false,
            "password" to "dav",
        )
        Api.failure("conn.test", args("input" to input), "unsupported")
        Api.failure("conn.save", args("input" to input), "unsupported")
        val https = input.withFields("https" to true)
        Api.failure("conn.test", args("input" to https), "network", "auth")
    }

    @Test
    fun connectionRemovalCleansFavoritesAndReportsJobs() = coreTest {
        val main = Servers.sftp()
        val folderName = Fixture.unique("bereinigen")
        Api.mkdir(main.text("location"), folderName)
        val extra = Api.obj(
            "conn.save",
            args("input" to Servers.sftpInput(root = TaskArgs.get("seSftpRoot").trimEnd('/') + "/" + folderName)),
        )
        assertTrue(extra.text("id") != main.text("id"))
        val location = extra.text("location")
        assertTrue(Api.obj("loc.toggleFavorite", args("location" to location)).bool("favorite"))
        val local = Fixture.dir(Volumes.primary(), "remote", "cleanup")
        val jobName = Fixture.unique("Aufräum-Job")
        val job = SyncJobs.save("name" to jobName, "source" to local.absolutePath, "target" to location, "enabled" to false)
        val report = Api.obj("conn.delete", args("id" to extra.text("id")))
        assertTrue("Favorit nicht entfernt: $report", report.int("removedFavorites") >= 1)
        assertTrue("Job nicht gemeldet: $report", jobName in report.texts("orphanedJobs"))
        assertFalse(Api.objects("conn.list").any { it.text("id") == extra.text("id") })
        assertFalse(Api.obj("loc.isFavorite", args("location" to location)).bool("favorite"))
        // Jobs are only reported, never deleted (desktop rule).
        SyncJobs.byId(job.text("id"))
        Api.call("sync.delete", args("id" to job.text("id")))
    }
}
