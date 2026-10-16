package app.smartexplorer.android.task

import android.os.Bundle
import android.os.Process
import android.util.Base64
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.smartexplorer.android.core.Core
import java.io.File
import java.io.FileOutputStream
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The secret store across an app update (native/luecken-task-android.py, job `android-device`). The
 * host installs the published APK 0.5.180 first, then this test runs in three instrumentation runs:
 *
 *  - `old-prepare` inside the PUBLISHED app: creates the Share identity, saves an SFTP connection
 *    with its password (the published app pins the server's host key silently) and records what
 *    the published app keeps on disk: plaintext `SESEC01` secret records.
 *  - `update-check` after `adb install -r` of the candidate APK: every old record is sealed
 *    (`SESEC02`) under the Android Keystore protector, no byte of an old secret is left anywhere in
 *    the app's files, the Share identity and the saved connection (stored password and old pin) work.
 *  - `restart-check` after a force-stop: the same again in a third process, with the same data key.
 *
 * The old phase only uses the JSON API of the core (`Api`, `Servers`), whose shape the published
 * app shares with the candidate; the host reads the facts from the status stream (never a secret).
 */
@RunWith(AndroidJUnit4::class)
class LueckenUpgradeTaskTest {
    private val filesDir get() = appContext.filesDir.canonicalFile
    private val data get() = File(filesDir, "smart_explorer")
    private val store get() = File(data, "secrets-v1")
    private val pins get() = File(data, "known_hosts_sftp.txt")
    private val fixture get() = File(filesDir, "luecken-upgrade")
    private val session get() = File(fixture, "session.json")

    // "SESEC01\u0000" (plaintext, SHA-256 checksum) and "SESEC02\u0000" (sealed).
    private val legacyMagic = byteArrayOf(83, 69, 83, 69, 67, 48, 49, 0)
    private val sealedMagic = byteArrayOf(83, 69, 83, 69, 67, 48, 50, 0)

    // Declared on one line like SyncReliabilityTaskTest: the class list of android-task.yml
    // (task_eval.py) does not know this class, and its host script dies on an unlisted test class.
    @Test fun publishedUpgrade() = coreTest {
        when (TaskArgs.get("lueckenPhase")) {
            "old-prepare" -> oldPrepare()
            "update-check" -> afterUpdate("update-check")
            "restart-check" -> afterUpdate("restart-check")
            else -> throw AssertionError("Unknown lueckenPhase")
        }
    }

    private fun startsWith(bytes: ByteArray, magic: ByteArray): Boolean =
        bytes.size >= magic.size && magic.indices.all { bytes[it] == magic[it] }

    private fun contains(haystack: ByteArray, needle: ByteArray): Boolean {
        if (needle.isEmpty() || haystack.size < needle.size) return false
        outer@ for (start in 0..haystack.size - needle.size) {
            for (index in needle.indices) {
                if (haystack[start + index] != needle[index]) continue@outer
            }
            return true
        }
        return false
    }

    private fun records(): List<File> =
        (store.listFiles() ?: emptyArray()).filter { it.isFile && it.name.endsWith(".secret") }.sortedBy { it.name }

    /** Secret bytes of a plaintext record: magic 8, version 1, account digest 32, length 4 (big endian), secret, SHA-256 32. */
    private fun legacySecret(file: File): ByteArray? {
        val bytes = file.readBytes()
        if (!startsWith(bytes, legacyMagic)) return null
        val length = ((bytes[41].toInt() and 0xff) shl 24) or ((bytes[42].toInt() and 0xff) shl 16) or
            ((bytes[43].toInt() and 0xff) shl 8) or (bytes[44].toInt() and 0xff)
        assertEquals("Damaged plaintext record ${file.name}", 45 + length + 32, bytes.size)
        return bytes.copyOfRange(45, 45 + length)
    }

    /** The core's directory listing as JSON (no typed classes: the published app shares the JSON shape only). */
    private suspend fun list(location: String): JsonObject = Api.obj(
        "fs.list",
        args(
            "location" to location,
            "showHidden" to true,
            "filter" to null,
            "sort" to mapOf("key" to "name", "desc" to false, "dirsFirst" to true),
            "refresh" to true,
        ),
    )

    /** [record] goes to the session file (it may hold secrets of this test device), [facts] to the host's status stream. */
    private fun checkpoint(record: JsonObject, phase: String, facts: JsonObject) {
        fixture.mkdirs()
        FileOutputStream(session).use {
            it.write(record.toString().toByteArray())
            it.fd.sync()
        }
        val message = "LUECKEN_UPGRADE_MARKER " + facts.withFields("phase" to phase) + "\n"
        InstrumentationRegistry.getInstrumentation().sendStatus(0, Bundle().apply { putString("stream", message) })
    }

    private suspend fun oldPrepare() {
        val oldVersion = TaskArgs.get("lueckenOldVersion")
        assertEquals(oldVersion, appContext.packageManager.getPackageInfo(appContext.packageName, 0).versionName)
        assertFalse("Fixture already exists", fixture.exists())
        Api.call("share.setName", args("name" to "Luecken-Upgrade"))
        val identity = Api.obj("share.status").field("identity").obj()
        assertTrue("The published app has no Share identity: $identity", identity.text("deviceId").isNotEmpty())
        val connection = Servers.sftp()
        val location = connection.text("location")
        list(location) // a real connection with the saved password; the published app pins the host key here

        val password = TaskArgs.get("seSftpPass").toByteArray()
        val secrets = records().map { it.name to legacySecret(it) }
        assertTrue(
            "The published app keeps no plaintext secret records: ${store.list()?.toList()}",
            secrets.count { it.second != null } >= 2,
        )
        assertEquals("The published app holds sealed records", secrets.size, secrets.count { it.second != null })
        assertTrue("The saved password is no plaintext record", secrets.any { it.second?.contentEquals(password) == true })
        assertFalse("The published app has a key descriptor", File(store, "keyring-v2").exists())
        assertTrue("The published app pinned no host key", pins.isFile && pins.length() > 0)
        val needles = secrets.mapNotNull { it.second }.filter { it.size >= 12 }
        assertTrue("The password is not searchable", needles.any { it.contentEquals(password) })
        val facts = args(
            "pid" to Process.myPid(),
            "records" to secrets.size,
            "needleCount" to needles.size,
            "deviceId" to identity.text("deviceId"),
            "pinLines" to pins.readLines().size,
        )
        val record = JsonObject(
            facts + args(
                "identity" to identity,
                "location" to location,
                "pin" to pins.readText(),
                "needles" to needles.map { Base64.encodeToString(it, Base64.NO_WRAP) },
            ),
        )
        checkpoint(record, "old-prepare", facts)
    }

    private suspend fun afterUpdate(phase: String) {
        val record = Core.json.parseToJsonElement(session.readText()).obj()
        assertNotEquals("The old and the new app ran in one process", record.int("pid"), Process.myPid())

        val sealed = records()
        assertTrue("The upgrade lost secret records: ${sealed.size} < ${record.int("records")}", sealed.size >= record.int("records"))
        val plaintext = sealed.filter { startsWith(it.readBytes(), legacyMagic) }.map { it.name }
        assertEquals("Plaintext records after the upgrade: $plaintext", emptyList<String>(), plaintext)
        sealed.forEach { assertTrue("${it.name} is not a sealed record", startsWith(it.readBytes(), sealedMagic)) }
        val stages = (store.list() ?: emptyArray()).filter { it.startsWith(".stage") }
        assertEquals("Leftover stage files: $stages", emptyList<String>(), stages)

        val descriptor = Core.json.parseToJsonElement(File(store, "keyring-v2").readText()).obj()
        val protector = descriptor.text("protector")
        assertEquals("Wrong key protector", "android-keystore", protector)
        val keyId = descriptor.text("key_id")
        record.textOrNull("keyId")?.let { assertEquals("The data key changed", it, keyId) }
        val wrapped = File(appContext.noBackupFilesDir, "secrets-key-v2.bin")
        assertTrue("The Keystore-wrapped data key is missing", wrapped.isFile && wrapped.length() > 0)

        val needles = record.texts("needles").map { Base64.decode(it, Base64.NO_WRAP) }
        val found = filesDir.walkTopDown()
            .filter { it.isFile && !it.path.startsWith(fixture.path) && it.length() <= 32L * 1024 * 1024 }
            .filter { file ->
                val bytes = file.readBytes()
                needles.any { contains(bytes, it) }
            }
            .map { it.relativeTo(filesDir).path }
            .toList()
        assertEquals("Plaintext of the old secrets after the upgrade: $found", emptyList<String>(), found)

        val identity = Api.obj("share.status").field("identity").obj()
        val before = record.field("identity").obj()
        assertEquals("The Share identity changed", before.text("deviceId"), identity.text("deviceId"))
        for (key in listOf("fingerprint", "directCode")) {
            assertEquals("$key changed", before.textOrNull(key), identity.textOrNull(key))
        }

        val location = record.text("location")
        val saved = Api.objects("conn.list").map { it.text("location") }
        assertTrue("The saved connection disappeared: $saved", location in saved)
        list(location) // the stored password and the pin of the published app: no password question, no host-key question
        assertEquals("The pin of the published app changed", record.text("pin"), pins.readText())

        val facts = args(
            "pid" to Process.myPid(),
            "protector" to protector,
            "keyId" to keyId,
            "records" to sealed.size,
            "plaintext" to plaintext.size,
            "stageFiles" to stages.size,
            "found" to found.size,
            "deviceId" to identity.text("deviceId"),
            "listed" to true,
        )
        checkpoint(JsonObject(record + facts), phase, facts)
    }
}
