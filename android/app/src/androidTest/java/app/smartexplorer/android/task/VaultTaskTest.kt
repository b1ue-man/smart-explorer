package app.smartexplorer.android.task

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.text.AnnotatedString
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.api.SyncApi
import app.smartexplorer.android.api.SyncJob
import app.smartexplorer.android.api.VaultApi
import app.smartexplorer.android.api.VaultSite
import app.smartexplorer.android.system.HostMonitor
import app.smartexplorer.android.ui.sync.JobActions
import app.smartexplorer.android.ui.sync.JobCard
import app.smartexplorer.android.ui.vaults.VaultUnlockSheet
import app.smartexplorer.android.ui.vaults.VaultsHost
import java.io.File
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * VLT-M11 on the emulator (stage `vault_android_device`, T 542 and U31-16): vaults on Android
 * through the bridge `vault.*` and the screens of „Mehr → Tresore“. Every test creates its own
 * vault in the primary volume; the background lock is switched off where the test does not test it,
 * because the instrumented app runs without a visible activity.
 */
@RunWith(AndroidJUnit4::class)
class VaultTaskTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val password = "Tresor-Passwort-1"

    /** The password field holds nothing (U31-14). */
    private val emptyField = SemanticsMatcher.expectValue(SemanticsProperties.EditableText, AnnotatedString(""))

    /** Creates, enters and unlocks a vault named [label]; background lock off. */
    private suspend fun vault(label: String): JsonObject {
        val dir = Fixture.dir(Volumes.primary(), "vault", Fixture.unique(label))
        val created = Api.runTask("vault.create", args("location" to dir.absolutePath, "label" to label, "password" to password)).resultObj()
        assertTrue("angelegt und entsperrt: $created", created.bool("unlocked"))
        Api.call("vault.setSettings", args("site" to created.text("site"), "lockOnBackgroundMin" to null))
        return created
    }

    private suspend fun state(site: String): String =
        Api.obj("vault.list").objects("vaults").flatMap { it.objects("sites") }.first { it.text("site") == site }.text("state")

    private suspend fun write(location: String, name: String, text: String) {
        Api.attempt("fs.delete", args("locations" to listOf("$location/$name"), "permanent" to true))
        val local = Fixture.write(File(Fixture.dir(Volumes.primary(), "vault-in", Fixture.unique("in")), name), text)
        Api.copy(listOf(local.absolutePath), location)
    }

    private suspend fun read(location: String, name: String): String {
        val out = Fixture.dir(Volumes.primary(), "vault-out", Fixture.unique("out"))
        Api.copy(listOf("$location/$name"), out.absolutePath)
        return File(out, name).readText()
    }

    private suspend fun unlock(site: String, memAvail: Long = 2L shl 30, memThreshold: Long = 0) = Api.await(
        Api.start("vault.unlock", args("site" to site, "password" to password, "memAvail" to memAvail, "memThreshold" to memThreshold)),
    )

    @Test
    fun vault_task_android_create_unlock_memory_write_read() = coreTest {
        val site = vault("Anlegen").text("site")
        write("vault://$site", "Notiz.txt", "geheim")
        assertTrue(Api.names("vault://$site/").contains("Notiz.txt"))
        assertEquals("geheim", read("vault://$site", "Notiz.txt"))
        Api.call("vault.lock", args("site" to site))
        assertEquals("locked", state(site))
        // Entsperren mit Speicherschwelle: too little memory for scrypt is refused with its code.
        val starved = unlock(site, memAvail = 1L shl 20, memThreshold = 0)
        assertEquals("failed", starved.state)
        assertEquals("vault_memory", starved.resultObj().text("code"))
        assertEquals("done", unlock(site).state)
        assertEquals("unlocked", state(site))
        // S1: no saved or remembered cleartext password on Android.
        Api.failure("vault.unlock", args("site" to site, "saved" to true), "invalid")
        Api.failure("vault.unlock", args("site" to site, "password" to password, "remember" to true), "invalid")
    }

    @Test
    fun vault_task_android_screen_off_locks() = coreTest {
        val site = vault("Bildschirm").text("site")
        try {
            SyncApi.hostState(HostMonitor.measure(appContext).copy(screenOn = false))
            waitFor("gesperrt nach Bildschirm aus", 20_000) { state(site) == "locked" }
        } finally {
            SyncApi.hostState(HostMonitor.measure(appContext).copy(screenOn = true))
        }
    }

    @Test
    fun vault_task_android_background_one_minute_locks() = coreTest {
        val site = vault("Hintergrund").text("site")
        Api.call("vault.setSettings", args("site" to site, "lockOnBackgroundMin" to 1))
        SyncApi.hostState(HostMonitor.measure(appContext).copy(foreground = false, screenOn = true))
        assertEquals("unlocked", state(site))
        waitFor("gesperrt nach 1 Minute im Hintergrund", 120_000) { state(site) == "locked" }
    }

    /** A vault, its replica job and a conflict on [names]; returns site A, site B and the job. */
    private suspend fun conflicted(label: String, vararg names: String): Triple<String, String, String> {
        val a = vault(label).text("site")
        names.forEach { write("vault://$a", it, "Ausgang $it") }
        val created = Api.obj("vault.syncCreate", args("site" to a, "target" to "replica", "direction" to "both"))
        val job = created.text("job")
        val b = created.text("siteB")
        Api.runTask("sync.run", args("id" to job))
        assertEquals("done", unlock(b).state)
        Api.call("vault.setSettings", args("site" to b, "lockOnBackgroundMin" to null))
        names.forEach {
            write("vault://$a", it, "A hat $it geändert")
            write("vault://$b", it, "B hat $it geändert")
        }
        Api.runTask("sync.run", args("id" to job))
        return Triple(a, b, job)
    }

    @Test
    fun vault_task_android_sync_conflict_compare_keep_both() = coreTest {
        val (_, _, job) = conflicted("Konflikt", "Bericht.txt")
        val head = Api.runTask("vault.compare", args("job" to job)).resultObj()
        assertTrue("Konflikt im Vergleich: $head", head.obj().field("counts").obj().int("conflict") >= 1)
        val page = Api.obj("vault.compareItems", args("list" to head.text("list"), "revision" to head.long("revision"), "group" to "conflict"))
        val item = page.objects("items").first()
        // Vergleich mit Klartextnamen.
        assertEquals("/Bericht.txt", item.text("path"))
        assertTrue(item.texts("actions").contains("keep-both"))
        val resolved = Api.runTask("vault.resolve", args("job" to job, "item" to item.text("item"), "action" to "keep-both", "confirmed" to true)).resultObj()
        assertEquals(1, resolved.int("resolved"))
    }

    @Test
    fun vault_task_android_notification_has_no_names() = coreTest {
        val (a, _, job) = conflicted("Meldung", "Geheimplan.txt")
        val label = Api.obj("vault.show", args("site" to a)).text("label")
        var text = ""
        waitFor("Benachrichtigung der Tresor-Konflikte", 30_000) {
            val posted = appContext.getSystemService(android.app.NotificationManager::class.java)
                ?.activeNotifications
                ?.firstOrNull { it.tag == "sync-job:vault:$job" }
            text = posted?.notification?.extras?.getCharSequence(android.app.Notification.EXTRA_TEXT)?.toString().orEmpty()
            posted != null
        }
        assertEquals("1 Konflikte in $label.", text)
        assertFalse(text.contains("Geheimplan"))
    }

    @Test
    fun vault_task_android_every_bridge_method_once() = coreTest {
        val called = mutableSetOf<String>()
        suspend fun call(method: String, vararg pairs: Pair<String, Any?>): JsonObject {
            called += method
            return Api.call(method, args(*pairs)).obj()
        }
        suspend fun task(method: String, vararg pairs: Pair<String, Any?>): JsonObject {
            called += method
            return Api.runTask(method, args(*pairs)).resultObj()
        }
        val (a, b, job) = conflicted("Methoden", "Text.txt", "Zwei.txt", "Drei.txt")
        called += setOf("vault.create", "vault.setSettings", "vault.syncCreate", "vault.unlock")
        call("vault.list")
        val shown = call("vault.show", "site" to a)
        assertFalse(shown.bool("recoveryKeySupported"))
        val location = shown.text("location")
        assertTrue(call("vault.detect", "location" to location).bool("isVault"))
        val head = task("vault.compare", "job" to job)
        val items = call("vault.compareItems", "list" to head.text("list"), "revision" to head.long("revision"), "group" to "conflict").objects("items")
        val text = items.first { it.text("path") == "/Text.txt" }
        val pair = call("vault.compareText", "job" to job, "item" to text.text("item"))
        assertTrue(pair.array("rows").isNotEmpty())
        task("vault.mergeApply", "job" to job, "item" to text.text("item"), "content" to "zusammengeführt\n")
        task("vault.resolve", "job" to job, "item" to items.first { it.text("path") == "/Zwei.txt" }.text("item"), "action" to "keep-a", "confirmed" to true)
        val all = task("vault.resolveAll", "job" to job, "action" to "keep-b", "confirmed" to true)
        assertTrue("Drei.txt aufgelöst: $all", all.int("resolved") >= 1)
        task("vault.check", "site" to a)
        // S1: only an envelope of the app is stored, and only for an unlocked site.
        val envelope = "android-kw1:" + "A".repeat(43)
        Api.failure("vault.rememberSaved", args("site" to a, "sealed" to "Klartext-Passwort"), "invalid")
        call("vault.rememberSaved", "site" to a, "sealed" to envelope)
        assertEquals(envelope, call("vault.savedSecret", "site" to a).text("sealed"))
        call("vault.forgetPassword", "site" to a)
        Api.failure("vault.savedSecret", args("site" to a), "vault_no_saved_password")
        task("vault.changePassword", "site" to b, "oldPassword" to password, "newPassword" to "Neues-Passwort-2")
        call("vault.lock", "site" to a)
        call("vault.lockAll")
        val locationB = call("vault.show", "site" to b).text("location")
        Api.call("sync.delete", args("id" to job))
        call("vault.remove", "site" to b)
        val again = call("vault.add", "location" to locationB, "label" to "Wieder eingetragen")
        assertTrue(again.text("site").isNotBlank())
        assertEquals("locked", state(again.text("site")))
        assertEquals(called.toString(), 22, called.size)
    }

    @Test
    fun vault_task_android_password_not_saved_state() {
        val site = VaultSite(site = "0123456789abcdef0123456789abcdef", label = "Wiederherstellung")
        val restore = StateRestorationTester(compose)
        restore.setContent { VaultUnlockSheet(site, onDismiss = {}, onUnlocked = {}) }
        compose.onNodeWithTag("vault-password").performTextInput("geheim-123")
        restore.emulateSavedInstanceStateRestore()
        compose.onNodeWithTag("vault-password").assert(emptyField)
    }

    @Test
    fun vault_task_android_unlock_wrong_password_clears_field() {
        val created = runBlocking { coreTestValue { vault("Falsch") } }
        val site = runBlocking { VaultApi.show(created.text("site")) }
        runBlocking { VaultApi.lock(site.site) }
        compose.setContent { VaultUnlockSheet(site.copy(state = "locked"), onDismiss = {}, onUnlocked = {}) }
        compose.onNodeWithTag("vault-password").performTextInput("falsch-falsch")
        compose.onNodeWithTag("vault-unlock").performClick()
        compose.waitUntil(60_000) { compose.onAllNodes(hasText("Passwort falsch")).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("vault-password").assert(emptyField)
    }

    @Test
    fun vault_task_android_lock_returns_to_list() {
        val site = runBlocking { coreTestValue { vault("Rückkehr") }.text("site") }
        compose.setContent { VaultsHost(onBack = {}, compareJob = null, onCompareOpened = {}) }
        // The list holds the vaults of every earlier test of this class, oldest first (`registry_store.rs`), and the
        // LazyColumn composes only the rows on screen: the new row is scrolled to, as the person would scroll.
        compose.waitUntil(30_000) {
            runCatching { compose.onNodeWithTag("vault-list").performScrollToNode(hasTestTag("vault-site-$site")) }.isSuccess
        }
        compose.onNode(hasContentDescription("Weitere Aktionen") and hasAnyAncestor(hasTestTag("vault-site-$site")), useUnmergedTree = true).performClick()
        compose.onNodeWithText("Tresor prüfen").performClick()
        compose.waitUntil(30_000) { compose.onAllNodes(hasTestTag("vault-list")).fetchSemanticsNodes().isEmpty() }
        runBlocking { VaultApi.lock(site) }
        compose.waitUntil(30_000) { compose.onAllNodes(hasTestTag("vault-list")).fetchSemanticsNodes().isNotEmpty() }
    }

    @Test
    fun vault_task_android_job_card_has_compare() {
        val job = SyncJob(id = "vaultjob1", name = "Tresor Privat", source = "vault-store://0123456789abcdef0123456789abcdef", target = "vault-store://fedcba9876543210fedcba9876543210")
        val actions = JobActions({}, {}, {}, {}, {}, {}, {}, {})
        compose.setContent { JobCard(job, emptyList(), running = false, runTask = null, daemonRuns = false, actions = actions) }
        compose.onNodeWithTag("vault-compare-vaultjob1").assertExists()
    }

    /** Runs [block] like [coreTest] and returns its value (Compose tests need it on this thread). */
    private suspend fun <T> coreTestValue(block: suspend () -> T): T {
        Api.awaitReady()
        return block()
    }
}
