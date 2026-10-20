package app.smartexplorer.android.task

import android.net.Uri
import android.provider.DocumentsContract
import android.provider.MediaStore
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.system.documents.DocumentIds
import app.smartexplorer.android.system.documents.SeDocumentsProvider
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import kotlinx.coroutines.delay
import kotlinx.serialization.json.JsonObject
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * DRV-M8 on the emulator (strand S2 C3): the documents provider of this app against the servers
 * of the runner, through the platform's own `ContentResolver` and proxy descriptors. The suite
 * shares the app's UID, so it reaches the provider without the picker; what other apps see is
 * the same provider. Settings are reset after each test (nothing released).
 */
@RunWith(AndroidJUnit4::class)
class DriveTaskProviderDeviceTest {
    private val authority get() = DocumentIds.authority(appContext)

    @After
    fun reset() = coreTest {
        Api.call("docs.settings.set", args("provider" to false, "devices" to false, "connections" to emptyList<String>(), "vaults" to emptyList<String>()))
        SeDocumentsProvider.setEnabled(appContext, false)
    }

    /** Releases [connection] and returns the document ID of its root. */
    private suspend fun release(connection: JsonObject): String {
        val id = connection.text("id")
        Api.call("docs.settings.set", args("provider" to true, "connections" to listOf(id)))
        SeDocumentsProvider.setEnabled(appContext, true)
        return "conn:" + id.toByteArray().joinToString("") { "%02x".format(it.toInt() and 0xff) }
    }

    /** The document ID of a remote [location] below the [connection]'s root [rootId]. */
    private fun docId(rootId: String, connection: JsonObject, location: String): String =
        rootId + "/" + location.removePrefix(connection.text("location")).trim('/')

    private fun uri(id: String): Uri = DocumentsContract.buildDocumentUri(authority, id)

    private suspend fun children(id: String): List<String> {
        val uri = DocumentsContract.buildChildDocumentsUri(authority, id)
        // A cold folder answers EXTRA_LOADING first; the listing follows.
        repeat(60) {
            appContext.contentResolver.query(uri, null, null, null, null)!!.use { cursor ->
                if (!cursor.extras.getBoolean(DocumentsContract.EXTRA_LOADING)) {
                    return buildList { while (cursor.moveToNext()) add(cursor.getString(cursor.getColumnIndexOrThrow(DocumentsContract.Document.COLUMN_DISPLAY_NAME))) }
                }
            }
            delay(500)
        }
        throw AssertionError("Ordner $id lädt nach 30 s noch")
    }

    /** The content of [name] in [remote] as the server has it now; null when it is not there. */
    private suspend fun remoteText(remote: String, name: String): String? {
        val back = Fixture.dir(Volumes.primary(), "docs", "back")
        Api.attempt("fs.stat", args("location" to "$remote/$name")).getOrNull() ?: return null
        val started = Api.attempt(
            "fs.transfer",
            args("sources" to listOf("$remote/$name"), "targetDir" to back.absolutePath, "mode" to "copy", "conflict" to "keepBoth"),
        ).getOrNull() ?: return null
        Api.await(started.obj().text("taskId"))
        return File(back, name).takeIf { it.exists() }?.readText()
    }

    @Test
    fun drive_task_android_open_read_seek_large_file_range() = coreTest {
        val connection = Servers.sftp()
        val root = release(connection)
        val remote = Servers.freshDir(connection, "docs-read")
        val local = Fixture.big(File(Fixture.dir(Volumes.primary(), "docs", "read"), "gross.bin"), 24, 23)
        Api.copy(listOf(local.absolutePath), remote)
        val expected = local.readBytes()
        appContext.contentResolver.openFileDescriptor(uri(docId(root, connection, "$remote/gross.bin")), "r")!!.use { pfd ->
            assertEquals(expected.size.toLong(), pfd.statSize)
            FileInputStream(pfd.fileDescriptor).channel.use { channel ->
                for (offset in listOf(20L * 1024 * 1024, 7L, 13L * 1024 * 1024 + 5, 0L)) {
                    val buffer = java.nio.ByteBuffer.allocate(64 * 1024)
                    channel.position(offset)
                    val read = channel.read(buffer)
                    assertTrue(read > 0)
                    assertArrayEquals(expected.copyOfRange(offset.toInt(), offset.toInt() + read), buffer.array().copyOf(read))
                }
            }
        }
    }

    @Test
    fun drive_task_android_write_close_uploads() = coreTest {
        val connection = Servers.sftp()
        val root = release(connection)
        val remote = Servers.freshDir(connection, "docs-write")
        val created = DocumentsContract.createDocument(appContext.contentResolver, uri(docId(root, connection, remote)), "text/plain", "neu.txt")!!
        appContext.contentResolver.openFileDescriptor(created, "wt")!!.use { pfd ->
            FileOutputStream(pfd.fileDescriptor).use { it.write("aus einer anderen App".toByteArray()) }
        }
        waitFor("Hochladen nach dem Schließen", 120_000) { remoteText(remote, "neu.txt") == "aus einer anderen App" }
    }

    @Test
    fun drive_task_android_process_kill_retries_pending() = coreTest {
        // A change that is in the journal but not published yet (descriptor still open, so no
        // release published it) goes out through `docs.retryPending`, the path the commit worker
        // takes after a process end. The process end itself cannot happen inside instrumentation.
        val connection = Servers.sftp()
        val root = release(connection)
        val remote = Servers.freshDir(connection, "docs-retry")
        val created = DocumentsContract.createDocument(appContext.contentResolver, uri(docId(root, connection, remote)), "text/plain", "offen.txt")!!
        appContext.contentResolver.openFileDescriptor(created, "w")!!.use { pfd ->
            FileOutputStream(pfd.fileDescriptor).let { out ->
                out.write("Journal".toByteArray())
                out.flush()
                assertTrue("Nichts im Journal", Api.obj("docs.pending").int("pending") > 0)
                Api.call("docs.retryPending")
                waitFor("Wiederholung veröffentlicht", 60_000) { Api.obj("docs.pending").int("pending") == 0 }
            }
        }
        assertEquals("Journal", remoteText(remote, "offen.txt"))
    }

    @Test
    fun drive_task_android_tree_grant_persists_restart() = coreTest {
        // Restart of the provider's state: settings off and on drop every engine; the tree URI and
        // its document IDs stay the same (the IDs carry no session state).
        val connection = Servers.sftp()
        val root = release(connection)
        val remote = Servers.freshDir(connection, "docs-tree")
        val folder = docId(root, connection, remote)
        DocumentsContract.createDocument(appContext.contentResolver, uri(folder), "text/plain", "baum.txt")
        val tree = DocumentsContract.buildTreeDocumentUri(authority, folder)
        val first = children(folder)
        Api.call("docs.settings.set", args("provider" to false))
        Api.call("docs.settings.set", args("provider" to true))
        val again = DocumentsContract.buildChildDocumentsUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree))
        appContext.contentResolver.query(again, null, null, null, null)!!.close()
        assertEquals(first, children(folder))
        assertTrue(DocumentsContract.isChildDocument(appContext.contentResolver, uri(folder), uri("$folder/baum.txt")))
    }

    @Test
    fun drive_task_android_handler_thread_per_file() = coreTest {
        val connection = Servers.sftp()
        val root = release(connection)
        val remote = Servers.freshDir(connection, "docs-threads")
        val folder = uri(docId(root, connection, remote))
        val a = DocumentsContract.createDocument(appContext.contentResolver, folder, "text/plain", "a.txt")!!
        val b = DocumentsContract.createDocument(appContext.contentResolver, folder, "text/plain", "b.txt")!!
        fun docThreads() = Thread.getAllStackTraces().keys.filter { it.name.startsWith("se-doc-") && it.isAlive }.map { it.name }.toSet()
        val before = docThreads()
        appContext.contentResolver.openFileDescriptor(a, "r")!!.use {
            appContext.contentResolver.openFileDescriptor(b, "r")!!.use {
                assertEquals("Je Datei ein eigener HandlerThread", 2, (docThreads() - before).size)
            }
        }
        waitFor("HandlerThreads beendet", 10_000) { (docThreads() - before).isEmpty() }
    }

    @Test
    fun drive_task_android_smb_root_rw() = coreTest {
        val connection = Servers.smb()
        val root = release(connection)
        val remote = Servers.freshDir(connection, "docs-smb")
        val folder = uri(docId(root, connection, remote))
        val created = DocumentsContract.createDocument(appContext.contentResolver, folder, "text/plain", "smb.txt")!!
        appContext.contentResolver.openFileDescriptor(created, "wt")!!.use { pfd ->
            FileOutputStream(pfd.fileDescriptor).use { it.write("SMB".toByteArray()) }
        }
        waitFor("SMB veröffentlicht", 60_000) { Api.obj("docs.pending").int("pending") == 0 }
        appContext.contentResolver.openInputStream(created)!!.use { assertEquals("SMB", it.readBytes().decodeToString()) }
        val renamed = DocumentsContract.renameDocument(appContext.contentResolver, created, "umbenannt.txt")!!
        assertTrue("umbenannt.txt" in Api.names(remote))
        assertTrue(DocumentsContract.deleteDocument(appContext.contentResolver, renamed))
        assertFalse("umbenannt.txt" in Api.names(remote))
    }

    @Test
    fun drive_task_android_mediastore_has_no_provider_files() = coreTest {
        val connection = Servers.sftp()
        val root = release(connection)
        val remote = Servers.freshDir(connection, "docs-media")
        val name = Fixture.unique("docs-bild") + ".jpg"
        val created = DocumentsContract.createDocument(appContext.contentResolver, uri(docId(root, connection, remote)), "image/jpeg", name)!!
        appContext.contentResolver.openFileDescriptor(created, "wt")!!.use { pfd ->
            FileOutputStream(pfd.fileDescriptor).use { it.write(ByteArray(1024) { 7 }) }
        }
        delay(5_000)
        val files = MediaStore.Files.getContentUri(MediaStore.VOLUME_EXTERNAL)
        appContext.contentResolver.query(files, arrayOf(MediaStore.MediaColumns.DISPLAY_NAME), "${MediaStore.MediaColumns.DISPLAY_NAME} = ?", arrayOf(name), null)!!.use {
            assertEquals("MediaStore führt eine Datei des Anbieters", 0, it.count)
        }
    }
}
