package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.api.SHARE_DIRECT
import app.smartexplorer.android.api.SHARE_READ_ONLY
import app.smartexplorer.android.api.SHARE_READ_WRITE
import app.smartexplorer.android.api.ShareApi
import app.smartexplorer.android.api.SharePolicyApi
import app.smartexplorer.android.api.ShareRoom
import app.smartexplorer.android.api.ShareSecurityApi
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreException
import java.io.File
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonArray
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** RV1 JNI acceptance. Desktop identities/server/room are discovered by the owning runner. */
@RunWith(AndroidJUnit4::class)
class ReviewShareTaskTest {
    @Test
    fun persistedRootRoomAndAccountRightsRejectStaleCas() = coreTest(10 * 60_000L) {
        scenario {
            val local = folder("Rechte ä + literal")
            val probe = Fixture.bytes(File(local, "unveraendert.bin"), 8192, 714)
            val hash = Fixture.sha256(probe)
            val path = export(SHARE_DIRECT, local)
            assertEquals(SHARE_READ_ONLY, root(SHARE_DIRECT, path).text("access"))
            assertFalse(root(SHARE_DIRECT, path).optionalBool("allow_system_writes"))
            val rootView = status().exports.direct.single { it.path == path }
            assertEquals(SHARE_READ_ONLY, rootView.access)
            assertEquals(false, rootView.allowSystemWrites)
            val made = Api.obj("share.createRoom", args("name" to token))
            val roomId = made.text("profileId").also { rooms += it }
            var currentRoom = room(roomId)
            assertTrue(currentRoom.members.isEmpty())
            assertFalse(currentRoom.policy!!.membersMayWrite)
            assertFalse(currentRoom.policy!!.confirmNewMembers)
            assertTrue(storedRoom(roomId).field("exports").obj().objects("roots").isEmpty())
            export(roomId, local)
            consume("share.setRoomPolicy") { SharePolicyApi.setRoomPolicy(currentRoom, membersMayWrite = true, confirmNewMembers = true) }
            currentRoom = room(roomId)
            assertTrue(currentRoom.policy!!.membersMayWrite)
            assertTrue(currentRoom.policy!!.confirmNewMembers)
            consume("share.setRoomPolicy") { SharePolicyApi.setRoomPolicy(currentRoom, confirmNewMembers = false) }
            assertTrue(storedRoom(roomId).field("policy").obj().bool("members_may_write"))
            assertFalse(storedRoom(roomId).field("policy").obj().bool("confirm_new_members"))
            assertEquals(SHARE_READ_ONLY, root(roomId, path).text("access"))
            assertEquals(SHARE_READ_ONLY, status().exports.rooms.getValue(roomId).single { it.path == path }.access)
            consume("share.setExportAccess") { SharePolicyApi.setExportAccess(SHARE_DIRECT, path, SHARE_READ_WRITE) }
            consume("share.setExportAccess") { SharePolicyApi.setExportAccess(SHARE_DIRECT, path, SHARE_READ_ONLY) }
            unchangedFailure("share.setExportAccess") {
                SharePolicyApi.setExportAccess(SHARE_DIRECT, path, SHARE_READ_WRITE, true, SHARE_READ_WRITE)
            }
            consume("share.setExportAccess") { SharePolicyApi.setExportAccess(SHARE_DIRECT, path, SHARE_READ_ONLY, true, SHARE_READ_ONLY) }
            assertEquals(SHARE_READ_ONLY, root(SHARE_DIRECT, path).text("access"))
            assertTrue(root(SHARE_DIRECT, path).optionalBool("allow_system_writes"))
            assertEquals(true, status().exports.direct.single { it.path == path }.allowSystemWrites)

            val saved = Api.obj("conn.save", args("input" to mapOf(
                "label" to token, "protocol" to "sftp", "host" to "$token.invalid", "port" to 22,
                "user" to "fixture", "root" to "/literal ä + root", "auth" to "password", "useAgent" to false,
            )))
            val account = saved.text("id").also { accounts += it }
            val choices = consume("share.connections") { SharePolicyApi.connections(SHARE_DIRECT) }
            assertEquals(account, choices.connections.single { it.label == token }.account)
            assertFalse(choices.connections.single { it.account == account }.shared)
            val committed = Api.obj("share.setConnectionExport", args("scope" to SHARE_DIRECT, "account" to account, "shared" to true))
            assertTrue(committed.bool("persisted"))
            assertTrue(committed.bool("changed"))
            assertEquals(SHARE_READ_ONLY, shared(account).text("access"))
            consume("share.setConnectionExport") { SharePolicyApi.setConnectionExport(SHARE_DIRECT, account, true, SHARE_READ_WRITE, true) }
            assertEquals(SHARE_READ_WRITE, shared(account).text("access"))
            consume("share.setConnectionExport") { SharePolicyApi.setConnectionExport(SHARE_DIRECT, account, false) }
            repeat(2) {
                unchangedFailure("share.setConnectionExport") {
                    SharePolicyApi.setConnectionExport(SHARE_DIRECT, account, true, SHARE_READ_WRITE, true)
                }
            }
            assertFalse(profiles().field("default_direct_exports").obj().optionalObjects("shared_connections").any { it.text("account") == account })
            assertFalse(consume("share.connections") { SharePolicyApi.connections(SHARE_DIRECT) }.sharedConnections.any { it.account == account })
            assertEquals(hash, Fixture.sha256(probe))
            assertNoExec()
            TaskReport.note("RV1-share-rights", "root=$path account=$account room=$roomId probeSha256=$hash; stale CAS refused")
        }
    }

    @Test
    fun contactAndRoomAdmissionRequireCurrentFullPins() = coreTest(10 * 60_000L) {
        scenario {
            val desktop = TaskArgs.get("seDesktopDevice")
            assertFalse(profiles().objects("direct_grants").any { it.text("device_id") == desktop })
            consume("share.setName") { ShareApi.setName(PHONE_NAME) }
            assertEquals(PHONE_NAME, status().identity.deviceName)
            val contact = consume("share.addDirect") { ShareApi.addDirect(TaskArgs.get("seDesktopDirectCode"), token) }
            contacts += contact
            assertFalse(profiles().objects("direct_contacts").single { it.text("id") == contact }.field("relation").obj().bool("share_back"))
            online()
            Api.call("share.requestAccess", args("contactId" to contact, "message" to "RV1 explicit fixture request"))
            waitFor("gezielte Desktopannahme und persistierte Kontaktpins", 120_000) {
                profiles().objects("direct_contacts").single { it.text("id") == contact }.let {
                    it.textOrNull("remote_device_id") == desktop && it.text("access_state") == "Accepted" &&
                        (!it.textOrNull("remote_public_key").isNullOrBlank() || !it.textOrNull("accepted_public_key").isNullOrBlank())
                }
            }
            consume("share.setShareBack") { SharePolicyApi.setShareBack(contact, true) }
            waitFor("aktueller lokaler Grant", 30_000) { status().writeGrants.orEmpty().any { it.deviceId == desktop && it.active } }
            quiet()
            val grant = status().writeGrants!!.single { it.deviceId == desktop }
            assertFalse(grant.write)
            assertEquals(grant.publicKey, storedGrant(desktop).text("public_key"))
            assertEquals(grant.nodeId, storedGrant(desktop).text("node_id"))
            assertEquals(grant.fingerprint, storedGrant(desktop).text("fingerprint"))
            val staleGrants = listOf(grant.copy(deviceId = desktop + "-stale"), grant.copy(publicKey = grant.publicKey + "-stale"),
                grant.copy(nodeId = grant.nodeId + "-stale"), grant.copy(fingerprint = grant.fingerprint + "-stale"))
            staleGrants.forEach { stale -> unchangedFailure("share.setContactWrite") { SharePolicyApi.setContactWrite(stale, true) } }
            consume("share.setContactWrite") { SharePolicyApi.setContactWrite(grant, true) }
            assertTrue(storedGrant(desktop).bool("write"))
            assertTrue(status().writeGrants!!.single { it.deviceId == desktop }.write)
            consume("share.withdrawGrant") { SharePolicyApi.withdrawGrant(grant) }
            assertEquals("Ignored", storedGrant(desktop).text("state"))
            assertFalse(status().writeGrants!!.single { it.deviceId == desktop }.active)
            unchangedFailure("share.setContactWrite") { SharePolicyApi.setContactWrite(grant, true) }
            consume("share.allowGrantAgain") { SharePolicyApi.allowGrantAgain(grant) }
            assertEquals("Accepted", storedGrant(desktop).text("state"))
            assertTrue(storedGrant(desktop).bool("write"))
            assertTrue(status().writeGrants!!.single { it.deviceId == desktop }.active)
            assertNoExec()

            val before = profiles().objects("rooms").map { it.text("id") }.toSet()
            val roomId = Api.obj("share.joinRoom", args("code" to TaskArgs.get("seRoomCode"), "name" to token)).text("profileId")
            assertFalse("Runnerraum gehört bereits zu einem anderen Szenario", roomId in before)
            rooms += roomId
            var joined = room(roomId)
            assertTrue("Pending muss vor der ersten Präsenz gewählt werden", joined.members.isEmpty())
            consume("share.setRoomPolicy") { SharePolicyApi.setRoomPolicy(joined, membersMayWrite = false, confirmNewMembers = true) }
            online()
            waitFor("echtes Desktopmitglied im neuen Raum", 120_000) { room(roomId).members.any { it.deviceId == desktop } }
            quiet()
            joined = room(roomId)
            val member = joined.members.single { it.deviceId == desktop }
            assertEquals("Pending", member.admission)
            assertTrue(member.blocked)
            assertTrue(!member.publicKey.isNullOrBlank() && member.nodeId != null && !member.fingerprint.isNullOrBlank())
            assertEquals(member.publicKey, storedMember(roomId, desktop).text("public_key"))
            assertEquals(member.nodeId, storedMember(roomId, desktop).text("node_id"))
            assertEquals(member.fingerprint, storedMember(roomId, desktop).text("fingerprint"))
            val staleMembers = listOf(member.copy(deviceId = desktop + "-stale"), member.copy(publicKey = member.publicKey + "-stale"),
                member.copy(nodeId = member.nodeId + "-stale"), member.copy(fingerprint = member.fingerprint + "-stale"))
            staleMembers.forEach { stale -> unchangedFailure("share.setRoomMember") { SharePolicyApi.setRoomMember(joined, stale, "admit") } }
            unchangedFailure("share.setRoomMember") { SharePolicyApi.setRoomMember(joined.copy(profileId = roomId + "-stale"), member, "admit") }
            unchangedFailure("share.setRoomMember") { SharePolicyApi.setRoomMember(joined.copy(roomId = joined.roomId + "-stale"), member, "admit") }
            consume("share.setRoomMember") { SharePolicyApi.setRoomMember(joined, member, "admit") }
            assertEquals("Admitted", storedMember(roomId, desktop).field("relation").obj().text("admission"))
            assertFalse(storedMember(roomId, desktop).bool("blocked"))
            assertEquals("Admitted", room(roomId).members.single { it.deviceId == desktop }.admission)
            consume("share.setRoomMember") { SharePolicyApi.setRoomMember(joined, member, "block") }
            assertTrue(storedMember(roomId, desktop).bool("blocked"))
            consume("share.setRoomMember") { SharePolicyApi.setRoomMember(joined, member, "allow") }
            assertFalse(storedMember(roomId, desktop).bool("blocked"))
            assertFalse(storedRoom(roomId).field("policy").obj().bool("members_may_write"))
            assertNoExec()
            TaskReport.note("RV1-share-pins", "desktop=$desktop room=$roomId; stale full pins refused, explicit admit/block/allow persisted")
        }
    }

    @Test
    fun tlsWeakPinConsentAndStoreFailuresRemainRetryable() = coreTest(10 * 60_000L) {
        scenario {
            val local = folder("Speicherretry")
            val path = export(SHARE_DIRECT, local)
            lockFault {
                unchangedFailure("share.setExportAccess") { SharePolicyApi.setExportAccess(SHARE_DIRECT, path, SHARE_READ_WRITE) }
            }
            consume("share.setExportAccess") { SharePolicyApi.setExportAccess(SHARE_DIRECT, path, SHARE_READ_WRITE) }
            assertEquals(SHARE_READ_WRITE, root(SHARE_DIRECT, path).text("access"))
            val encrypted = consume("share.setServer") { ShareSecurityApi.setServer("rv1-share.invalid:51820", false) }
            assertEquals("encrypted", encrypted.security)
            assertTrue(encrypted.server.startsWith("wss://"))
            assertFalse(encrypted.plaintext)
            assertArrayEquals(encrypted.server.toByteArray(Charsets.UTF_8), serverFile.readBytes())
            val serverBytes = serverFile.readBytes()
            failure("share.setServer", "invalid") { ShareApi.setServer("tcp://rv1-share.invalid:51820") }
            assertArrayEquals(serverBytes, serverFile.readBytes())
            val plaintext = consume("share.setServer") { ShareSecurityApi.setServer("tcp://rv1-share.invalid:51820", true) }
            assertEquals("plaintext", plaintext.security)
            assertTrue(plaintext.plaintext)
            assertArrayEquals(plaintext.server.toByteArray(Charsets.UTF_8), serverFile.readBytes())
            val plainBytes = serverFile.readBytes()
            failure("share.setServer", "invalid") { ShareSecurityApi.setServer("wss://rv1-share.invalid:51820,tcp://rv1-share.invalid:51820", true) }
            assertArrayEquals(plainBytes, serverFile.readBytes())
            online()
            val pin = consume("share.suggestPin") { ShareSecurityApi.suggestPin() }
            assertTrue("Native PIN muss aus sechs ASCII-Ziffern bestehen", pin.matches(Regex("[0-9]{6}")))
            for (allow in listOf(false, true)) {
                Api.failure("share.discoverable", args("target" to SHARE_DIRECT, "alias" to token, "pin" to "", "minutes" to 1, "allowWeakPin" to allow), "invalid")
            }
            failure("share.discoverable", "weak_pin") { ShareApi.discoverable(SHARE_DIRECT, token, "123456", 1) }
            failure("share.discoverable", "invalid") { ShareApi.discoverable(SHARE_DIRECT, token, pin, 31) }
            assertTrue(offer() == null)
            publishAndStop("123456", allowWeak = true)
            publishAndStop(pin, allowWeak = false)
            assertNoExec()
            TaskReport.note("RV1-share-security", "TLS/plaintext consent, actual weak/strong offers and lock-failure retry observed")
        }
    }

    private val dataDir get() = File(appContext.filesDir, "smart_explorer")
    private val profileFile get() = File(dataDir, "share_profiles.json")
    private val serverFile get() = File(dataDir, "share_server.txt")
    private fun profiles(): JsonObject = Core.json.parseToJsonElement(profileFile.readText(Charsets.UTF_8)).obj()
    private fun storedRoom(id: String) = profiles().objects("rooms").single { it.text("id") == id }
    private fun storedGrant(id: String) = profiles().objects("direct_grants").single { it.text("device_id") == id }
    private fun storedMember(room: String, device: String) = storedRoom(room).objects("members").single { it.text("device_id") == device }
    private fun root(scope: String, path: String): JsonObject {
        val config = if (scope == SHARE_DIRECT) profiles().field("default_direct_exports").obj() else storedRoom(scope).field("exports").obj()
        return config.objects("roots").single { it.text("path") == path }
    }
    private fun JsonObject.optionalBool(name: String) = if (name in this) bool(name) else false
    private fun JsonObject.optionalObjects(name: String): List<JsonObject> = this[name]?.let {
        (it as? JsonArray ?: throw AssertionError("$name muss Array sein")).map { value -> value.obj() }
    }.orEmpty()
    private fun shared(account: String) = profiles().field("default_direct_exports").obj().objects("shared_connections").single { it.text("account") == account }
    private suspend fun status() = consume("share.status") { ShareApi.status() }
    private suspend fun room(id: String): ShareRoom = status().rooms.single { it.profileId == id }
    private suspend fun quiet() {
        consume("share.setOnline") { ShareApi.setOnline(false) }
        waitFor("Share-Worker beendet", 30_000) { status().let { !it.running && !it.connected } }
    }
    private suspend fun online() {
        val saved = consume("share.setServer") { ShareSecurityApi.setServer(TaskArgs.get("seShareServer"), false) }
        assertEquals("Runner muss TLS mit passendem Pin bereitstellen", "encrypted", saved.security)
        assertFalse(saved.plaintext)
        consume("share.setOnline") { ShareApi.setOnline(true) }
        waitFor("TLS-Share-Verbindung des Fixtures", 90_000) { status().let { it.running && it.connected } }
    }
    private suspend fun <T> consume(method: String, block: suspend () -> T): T {
        try { return block().also { TaskReport.called(method, "ok") } }
        catch (e: CoreException) { TaskReport.called(method, "error:${e.kind}"); throw e }
    }
    private suspend fun failure(method: String, kind: String, block: suspend () -> Unit): CoreException {
        val error = try { consume(method, block); null } catch (e: CoreException) { e }
            ?: throw AssertionError("$method musste mit $kind scheitern")
        assertEquals(method, kind, error.kind)
        assertTrue("Retry braucht lesbaren Fehler", !error.message.isNullOrBlank())
        return error
    }
    private suspend fun unchangedFailure(method: String, block: suspend () -> Unit) {
        val before = profileFile.readBytes()
        failure(method, "invalid", block)
        assertArrayEquals("Abgelehnte $method darf keine Profilbytes ändern", before, profileFile.readBytes())
    }
    private suspend fun offer(): JsonObject? = Api.obj("share.status").field("discovery").obj()["offer"] as? JsonObject

    private suspend fun scenario(block: suspend Case.() -> Unit) {
        Api.obj("bg.ensureDaemon")
        Api.obj("share.connections", args("scope" to SHARE_DIRECT)) // Strict persisted loader, not a status-only baseline.
        val previousServer = consume("share.serverInfo") { ShareSecurityApi.serverInfo() }
        val initial = status()
        // A missing file has the real ShareProfiles::default() auto_connect=true.
        // quiet() is the first persisted mutation when the getter did not create a file.
        val previousOnline = if (profileFile.exists()) profiles().bool("auto_connect") else true
        val fixture = Case(previousServer.server, previousServer.plaintext, previousOnline, initial.identity.deviceName,
            initial.execTargets.associate { it.targetKey to it.enabled })
        var primary: Throwable? = null
        try { quiet(); fixture.block() }
        catch (e: Throwable) { primary = e; throw e }
        finally {
            try { withContext(NonCancellable) { withTimeout(90_000) { fixture.cleanup() } } }
            catch (e: Throwable) { if (primary != null) primary.addSuppressed(e) else throw e }
        }
    }

    private inner class Case(val previousServer: String, val previousPlaintext: Boolean, val previousOnline: Boolean, val previousName: String,
        val previousExec: Map<String, Boolean>) {
        val token = Fixture.unique("rv1-share")
        val rooms = mutableListOf<String>()
        val contacts = mutableListOf<String>()
        val accounts = mutableListOf<String>()
        val folders = mutableListOf<File>()
        val exports = mutableListOf<Pair<String, String>>()
        val aliases = mutableSetOf<String>()
        fun folder(label: String): File {
            val parent = folders.firstOrNull() ?: Fixture.dir(Volumes.primary(), token).also { folders += it }
            return File(parent, label).also { assertTrue(it.mkdirs()) }
        }
        suspend fun assertNoExec() {
            status().execTargets.forEach { target ->
                assertEquals("Fixture verändert kein Exec: ${target.targetKey}", previousExec[target.targetKey] ?: false, target.enabled)
            }
        }
        suspend fun export(scope: String, directory: File): String {
            Api.call("share.addExport", args("scope" to scope, "path" to directory.absolutePath, "label" to token))
            val config = if (scope == SHARE_DIRECT) profiles().field("default_direct_exports").obj() else storedRoom(scope).field("exports").obj()
            val path = config.objects("roots").single { it.text("label") == token }.text("path")
            exports += scope to path
            assertEquals(directory.canonicalPath, path)
            return path
        }
        suspend fun publishAndStop(pin: String, allowWeak: Boolean) {
            val alias = "$token-${if (allowWeak) "weak" else "strong"}".also { aliases += it }
            val started = System.currentTimeMillis()
            consume("share.discoverable") { ShareApi.discoverable(SHARE_DIRECT, alias, pin, 1, allowWeak) }
            var actual: JsonObject? = null
            waitFor("tatsächlich publiziertes Discovery-Angebot", 90_000) { actual = offer(); actual?.text("alias") == alias }
            val published = actual!!
            assertEquals(SHARE_DIRECT, published.text("target"))
            assertTrue(published.long("untilMs") > started)
            assertTrue("Einminütiges Angebot überschreitet seinen Rahmen", published.long("untilMs") <= System.currentTimeMillis() + 70_000)
            consume("share.stopDiscoverable") { ShareApi.stopDiscoverable(published.text("offerId")) }
            waitFor("eigenes Discovery-Angebot beendet", 30_000) { offer()?.text("offerId") != published.text("offerId") }
        }
        suspend fun lockFault(block: suspend () -> Unit) {
            val lock = File(dataDir, "share_profiles.lock")
            val backup = File(dataDir, "$token.lock-backup")
            assertFalse(backup.exists())
            val existed = lock.exists()
            if (existed) { assertTrue(lock.isFile); assertTrue(lock.renameTo(backup)) }
            try {
                assertTrue("Lockfehler nicht eingerichtet", lock.mkdir())
                block()
            } finally {
                if (lock.isDirectory) assertTrue("Lockblockade nicht entfernt", lock.delete())
                if (existed) assertTrue("Original-Lock nicht restauriert", backup.renameTo(lock))
            }
        }
        suspend fun cleanup() {
            val errors = mutableListOf<Throwable>()
            suspend fun step(block: suspend () -> Unit) { try { block() } catch (e: Throwable) { errors += e } }
            step { offer()?.takeIf { it.text("alias") in aliases }?.let { Api.call("share.stopDiscoverable", args("offerId" to it.text("offerId"))) } }
            step { quiet() }
            exports.asReversed().forEach { (scope, path) -> step { Api.call("share.removeExport", args("scope" to scope, "path" to path)) } }
            rooms.asReversed().forEach { id -> step { Api.obj("share.removeRoom", args("profileId" to id)) } }
            contacts.asReversed().forEach { id -> step { Api.obj("share.removeDevice", args("contactId" to id)) } }
            accounts.asReversed().forEach { id -> step { Api.obj("conn.delete", args("id" to id)) } }
            step { consume("share.setServer") { ShareSecurityApi.setServer(previousServer, previousPlaintext) } }
            if (previousName.isNotBlank()) step { consume("share.setName") { ShareApi.setName(previousName) } }
            step { consume("share.setOnline") { ShareApi.setOnline(previousOnline) } }
            folders.asReversed().forEach { folder -> step { assertTrue("Fixtureordner nicht entfernt", !folder.exists() || folder.deleteRecursively()) } }
            if (errors.isNotEmpty()) throw AssertionError("RV1 Share-Cleanup unvollständig").also { error -> errors.forEach(error::addSuppressed) }
        }
    }

    private companion object { const val PHONE_NAME = "RV1-Android-Share" }
}
