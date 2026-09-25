package app.smartexplorer.android.ui.connections

import app.smartexplorer.android.api.Connection
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Connection form rules (spec F12): default ports, field errors and the `ConnectionInput` contract. */
class ConnectionDraftTest {
    @Test
    fun protocolSwitchReplacesOnlyTheDefaultPort() {
        val sftp = ConnectionDraft(host = "h")
        assertEquals("22", sftp.port)
        assertEquals("21", sftp.withProtocol("ftp").port)
        assertEquals("21", sftp.withProtocol("ftps").port)
        assertEquals("443", sftp.withProtocol("webdav").port)
        val custom = sftp.copy(port = "2222")
        assertEquals("2222", custom.withProtocol("ftp").port)
        val webdav = sftp.withProtocol("webdav")
        assertEquals("80", webdav.withHttps(false).port)
        assertEquals("443", webdav.withHttps(false).withHttps(true).port)
        assertEquals(22, ConnectionDraft.defaultPort("sftp", true))
        assertEquals(80, ConnectionDraft.defaultPort("webdav", false))
    }

    @Test
    fun fieldErrors() {
        val errors = ConnectionDraft(host = " ", port = "70000", auth = "key").errors()
        assertEquals(setOf(ConnField.Host, ConnField.Port, ConnField.KeyPath), errors.keys)
        // The remote agent is deployed over SFTP after login; it never replaces the key file.
        assertEquals(setOf(ConnField.KeyPath), ConnectionDraft(host = "h", auth = "key", useAgent = true).errors().keys)
        // Only SFTP knows key authentication.
        assertTrue(ConnectionDraft(host = "h", protocol = "ftp", port = "21", auth = "key").errors().isEmpty())
    }

    @Test
    fun inputFollowsTheContract() {
        val password = ConnectionDraft(host = " files.example ", user = " u ", root = " ").toInput("secret", "")
        assertEquals("files.example", password.label)
        assertEquals("files.example", password.host)
        assertEquals("/", password.root)
        assertEquals("password", password.auth)
        assertEquals("secret", password.password)
        assertNull(password.passphrase)
        assertNull(password.keyPath)
        assertFalse(password.https)
        // Empty secrets are not sent (an edit keeps the stored one).
        assertNull(ConnectionDraft(id = "sftp://u@h:22/", host = "h").toInput("", "").password)

        val key = ConnectionDraft(host = "h", auth = "key", keyPath = "/k/id").toInput("ignored", "phrase")
        assertEquals("key", key.auth)
        assertEquals("/k/id", key.keyPath)
        assertNull(key.password)
        assertEquals("phrase", key.passphrase)

        val webdav = ConnectionDraft(host = "dav", protocol = "webdav", port = "443").toInput("p", "")
        assertTrue(webdav.https)
        assertEquals(443, webdav.port)
        assertFalse(ConnectionDraft(host = "h", protocol = "ftp", port = "abc").toInput("p", "").https)
        assertEquals(21, ConnectionDraft(host = "h", protocol = "ftp", port = "abc").toInput("p", "").port)
    }

    @Test
    fun draftFromSavedConnection() {
        val connection = Connection(id = "ftp://u@h:0/", protocol = "ftp", host = "h", port = 0, user = "u", root = "")
        val draft = ConnectionDraft.from(connection)
        assertEquals("21", draft.port)
        assertEquals("/", draft.root)
        assertEquals("ftp://u@h:0/", draft.id)
        assertEquals("WebDAV", ConnectionDraft.protocolLabel("webdav"))
    }
}
