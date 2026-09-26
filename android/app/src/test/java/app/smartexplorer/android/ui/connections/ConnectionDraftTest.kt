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

    @Test
    fun smbUsesPort445AndOnlyPasswords() {
        val smb = ConnectionDraft(host = "nas").withProtocol("smb")
        assertEquals("445", smb.port)
        assertEquals(445, ConnectionDraft.defaultPort("smb", true))
        assertEquals("SMB", ConnectionDraft.protocolLabel("smb"))
        assertEquals("21", smb.withProtocol("ftp").port)
        val input = smb.copy(share = "daten", auth = "key", useAgent = true).toInput("pw", "phrase")
        assertEquals("password", input.auth)
        assertEquals("pw", input.password)
        assertNull(input.passphrase)
        assertFalse(input.useAgent)
        assertFalse(input.https)
    }

    @Test
    fun smbComposesShareStartFolderAndDomain() {
        val draft = ConnectionDraft(protocol = "smb", host = " nas ", port = "445", user = " anna ", share = " daten ", domain = " FIRMA ")
        val input = draft.copy(root = "projekte\\2026/").toInput("pw", "")
        assertEquals("smb", input.protocol)
        assertEquals("nas", input.host)
        assertEquals("/daten/projekte/2026", input.root)
        assertEquals("FIRMA\\anna", input.user)
        // Without a start folder the root is the share; without a domain the user stays as typed.
        val plain = draft.copy(domain = "", root = " ").toInput("", "")
        assertEquals("/daten", plain.root)
        assertEquals("anna", plain.user)
        assertNull(plain.password)
        // A user typed as DOMÄNE\benutzer is kept, not prefixed twice.
        assertEquals("AD\\bob", draft.copy(user = "AD\\bob").toInput("", "").user)
    }

    @Test
    fun smbShareIsRequired() {
        val missing = ConnectionDraft(protocol = "smb", host = "nas", port = "445").errors()
        assertEquals(setOf(ConnField.Share), missing.keys)
        val path = ConnectionDraft(protocol = "smb", host = "nas", port = "445", share = "a/b").errors()
        assertEquals(setOf(ConnField.Share), path.keys)
        assertTrue(ConnectionDraft(protocol = "smb", host = "nas", port = "445", share = "daten").errors().isEmpty())
        // Other protocols have no share.
        assertTrue(ConnectionDraft(host = "h").errors().isEmpty())
    }

    @Test
    fun smbDraftFromSavedConnectionSplitsShareAndDomain() {
        val saved = Connection(
            id = "smb://FIRMA\\anna@nas:1445/daten/projekte",
            protocol = "smb",
            host = "nas",
            port = 1445,
            user = "FIRMA\\anna",
            root = "/daten/projekte",
        )
        val draft = ConnectionDraft.from(saved)
        assertEquals("daten", draft.share)
        assertEquals("/projekte", draft.root)
        assertEquals("FIRMA", draft.domain)
        assertEquals("anna", draft.user)
        assertEquals("1445", draft.port)
        val again = draft.toInput("", "")
        assertEquals(saved.root, again.root)
        assertEquals(saved.user, again.user)
        val shareOnly = ConnectionDraft.from(saved.copy(root = "/daten", user = "anna"))
        assertEquals(Triple("daten", "/", ""), Triple(shareOnly.share, shareOnly.root, shareOnly.domain))
    }

    @Test
    fun pastedSmbAddressesFillTheSmbFields() {
        val unc = ConnectionDraft(host = "").withHostInput("\\\\nas\\daten\\projekte\\2026")
        assertEquals("smb", unc.protocol)
        assertEquals("445", unc.port)
        assertEquals(Triple("nas", "daten", "/projekte/2026"), Triple(unc.host, unc.share, unc.root))

        val url = ConnectionDraft(protocol = "smb", port = "445").withHostInput("smb://anna:geheim@nas.local:1445/daten/x/")
        assertEquals(Triple("nas.local", "1445", "anna"), Triple(url.host, url.port, url.user))
        assertEquals(Triple("daten", "/x", ""), Triple(url.share, url.root, url.domain))
        assertEquals("/daten/x", url.toInput("", "").root)
        // A domain in the URL form (`DOMÄNE;benutzer`) becomes DOMÄNE\benutzer.
        assertEquals("AD\\bob", ConnectionDraft().withHostInput("SMB://AD;bob@[fe80::1]/s").user)
        assertEquals("fe80::1", ConnectionDraft().withHostInput("smb://[fe80::1]:446/s").host)

        // A plain host stays a host; an address without a share keeps the share field.
        val plain = ConnectionDraft(host = "").withHostInput("files.example")
        assertEquals(Pair("sftp", "files.example"), Pair(plain.protocol, plain.host))
        val noShare = ConnectionDraft(protocol = "smb", port = "445", share = "alt").withHostInput("smb://nas")
        assertEquals(Pair("nas", "alt"), Pair(noShare.host, noShare.share))
        assertNull(SmbAddress.parse("smb://nas:99999/s"))
        assertNull(SmbAddress.parse("//"))

        // An address typed into the host field is split when the form is used.
        val typed = ConnectionDraft(protocol = "smb", port = "445", host = "\\\\nas\\daten").toInput("", "")
        assertEquals(Pair("nas", "/daten"), Pair(typed.host, typed.root))
        assertTrue(ConnectionDraft(protocol = "smb", port = "445", host = "\\\\nas\\daten").errors().isEmpty())
    }
}
