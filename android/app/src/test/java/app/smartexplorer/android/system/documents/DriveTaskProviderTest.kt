package app.smartexplorer.android.system.documents

import android.app.Application
import android.content.Context
import android.content.pm.PackageManager
import android.content.pm.ProviderInfo
import android.content.pm.ServiceInfo
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.provider.DocumentsContract.Root
import androidx.test.core.app.ApplicationProvider
import androidx.work.testing.TestListenableWorkerBuilder
import app.smartexplorer.android.api.DocsBackend
import app.smartexplorer.android.api.DocsChildren
import app.smartexplorer.android.api.DocsItem
import app.smartexplorer.android.api.DocsRoot
import app.smartexplorer.android.core.CoreException
import app.smartexplorer.android.core.DocsChanged
import app.smartexplorer.android.system.Notifications
import java.io.FileNotFoundException
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

private const val EACCES = 13L
private const val ENOENT = 2L
private const val SITE = "0123456789abcdef0123456789abcdef"
private const val VAULT = "vault:$SITE"
private const val DEVICE = "devices/direct~c1"

/**
 * The provider half of DRV-M8 (strand S2 C3) on the JVM: the Kotlin contract against a fake core
 * that answers like `docs.*` does. Which roots exist and what the engine does is the core's part
 * (`docs*.rs`, emulator signals of `DriveTaskProviderDeviceTest`). The application is the plain
 * [Application], so nothing of `SmartExplorerApp.onCreate` runs (NR N10).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = Application::class)
class DriveTaskProviderTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val fake = FakeBackend()
    private var started: Application? = null

    @Before
    fun install() {
        SeDocumentsProvider.backendFactory = { fake }
        SeDocumentsProvider.startCore = { started = it }
    }

    @After
    fun restore() {
        SeDocumentsProvider.backendFactory = { app.smartexplorer.android.api.NativeDocsBackend() }
        SeDocumentsProvider.startCore = { app.smartexplorer.android.core.Core.start(it) }
    }

    private fun provider(): SeDocumentsProvider {
        val info = ProviderInfo().apply {
            authority = DocumentIds.authority(context)
            exported = true
            grantUriPermissions = true
            readPermission = android.Manifest.permission.MANAGE_DOCUMENTS
            writePermission = android.Manifest.permission.MANAGE_DOCUMENTS
        }
        return Robolectric.buildContentProvider(SeDocumentsProvider::class.java).create(info).get()
    }

    private fun rootIds(provider: SeDocumentsProvider): List<String> =
        provider.queryRoots(null).use { cursor ->
            buildList { while (cursor.moveToNext()) add(cursor.getString(cursor.getColumnIndexOrThrow(Root.COLUMN_ROOT_ID))) }
        }

    private fun notified(): List<android.net.Uri> = shadowOf(context.contentResolver).notifiedUris.map { it.uri }

    @Test
    fun drive_task_android_roots_follow_settings() {
        val provider = provider()
        fake.roots = listOf(root("devices", "Smart Explorer – Geräte"))
        assertEquals(listOf("devices"), rootIds(provider))
        // The person releases a connection: the core answers it and announces the change.
        fake.roots = fake.roots + root("conn:736674703a2f2f", "Server", create = true)
        provider.onDocsChanged(context, DocsChanged(roots = true, documents = emptyList(), commitFailed = false))
        assertTrue(DocumentsContract.buildRootsUri(DocumentIds.authority(context)) in notified())
        assertEquals(listOf("devices", "conn:736674703a2f2f"), rootIds(provider))
        // The main switch turns the component off, so no other app can ask at all.
        SeDocumentsProvider.setEnabled(context, false)
        val component = android.content.ComponentName(context, SeDocumentsProvider::class.java)
        assertEquals(PackageManager.COMPONENT_ENABLED_STATE_DISABLED, context.packageManager.getComponentEnabledSetting(component))
        SeDocumentsProvider.setEnabled(context, true)
        assertEquals(PackageManager.COMPONENT_ENABLED_STATE_ENABLED, context.packageManager.getComponentEnabledSetting(component))
    }

    @Test
    fun drive_task_android_locked_vault_has_no_root() {
        val provider = provider()
        fake.roots = listOf(root(VAULT, "Akten (Tresor)", create = true))
        assertEquals(listOf(VAULT), rootIds(provider))
        // Lock: the core drops the root and revokes its descriptors (EACCES from then on).
        fake.roots = emptyList()
        fake.openResult = -EACCES
        provider.onDocsChanged(context, DocsChanged(roots = true, documents = emptyList(), commitFailed = false))
        assertTrue(DocumentsContract.buildRootsUri(DocumentIds.authority(context)) in notified())
        assertEquals(emptyList<String>(), rootIds(provider))
        try {
            provider.openDocument("$VAULT/a.txt", "r", null)
            fail("Ein gesperrter Tresor darf nichts öffnen")
        } catch (expected: FileNotFoundException) {
            assertTrue(expected.message.orEmpty().contains("13"))
        }
    }

    @Test
    fun drive_task_android_is_child_without_io() {
        val provider = provider()
        fake.failEveryCall = true
        assertTrue(provider.isChildDocument("gdrive", "gdrive/a/b"))
        assertTrue(provider.isChildDocument("$DEVICE/Dokumente", "$DEVICE/Dokumente/x.txt"))
        assertFalse(provider.isChildDocument("gdrive/a", "gdrive/ab"))
        assertFalse(provider.isChildDocument("gdrive", "$VAULT/a"))
        assertFalse(provider.isChildDocument("gdrive/a", "gdrive/a/../../b"))
        assertEquals(0, fake.calls)
    }

    @Test
    fun drive_task_android_children_loading_then_notify() {
        val provider = provider()
        val parent = "gdrive/Ordner"
        fake.listing = DocsChildren(loading = true)
        provider.queryChildDocuments(parent, null, null as String?).use { cursor ->
            assertTrue(cursor.extras.getBoolean(DocumentsContract.EXTRA_LOADING))
            assertEquals(DocumentsContract.buildChildDocumentsUri(DocumentIds.authority(context), parent), cursor.notificationUri)
        }
        fake.listing = DocsChildren(children = listOf(DocsItem("$parent/a.txt", "a.txt", size = 3)))
        provider.onDocsChanged(context, DocsChanged(roots = false, documents = listOf(parent), commitFailed = false))
        assertTrue(DocumentsContract.buildChildDocumentsUri(DocumentIds.authority(context), parent) in notified())
        provider.queryChildDocuments(parent, null, null as String?).use { cursor ->
            assertFalse(cursor.extras.getBoolean(DocumentsContract.EXTRA_LOADING))
            assertEquals(1, cursor.count)
        }
    }

    @Test
    fun drive_task_android_offline_device_extra_error() {
        val provider = provider()
        fake.listing = DocsChildren(error = "Gerät nicht erreichbar")
        provider.queryChildDocuments(DEVICE, null, null as String?).use { cursor ->
            assertEquals("Gerät nicht erreichbar", cursor.extras.getString(DocumentsContract.EXTRA_ERROR))
            assertEquals(0, cursor.count)
        }
    }

    @Test
    fun drive_task_android_mode_wt_truncates_w_not() {
        assertEquals(DocumentIds.OpenMode(write = true, truncate = true), DocumentIds.openMode("wt"))
        assertEquals(DocumentIds.OpenMode(write = true, truncate = true), DocumentIds.openMode("rwt"))
        assertEquals(DocumentIds.OpenMode(write = true, truncate = false), DocumentIds.openMode("w"))
        assertEquals(DocumentIds.OpenMode(write = true, truncate = false), DocumentIds.openMode("rw"))
        assertEquals(DocumentIds.OpenMode(write = false, truncate = false), DocumentIds.openMode("r"))
        // The mode reaches the core unchanged; the core truncates only for `t`.
        val provider = provider()
        fake.openResult = -ENOENT
        for (mode in listOf("wt", "w")) {
            try {
                provider.openDocument("gdrive/a.txt", mode, null)
            } catch (expected: FileNotFoundException) {
            }
        }
        assertEquals(listOf("wt", "w"), fake.openedModes)
    }

    @Test
    fun drive_task_android_append_rejected() {
        val provider = provider()
        for (mode in listOf("wa", "rwa")) {
            try {
                provider.openDocument("gdrive/a.txt", mode, null)
                fail("$mode hätte abgelehnt werden müssen")
            } catch (expected: IllegalArgumentException) {
            }
        }
        assertEquals(null, DocumentIds.openMode("wa"))
        assertEquals(emptyList<String>(), fake.openedModes)
    }

    @Test
    fun drive_task_android_ids_stable_after_device_rename() {
        val provider = provider()
        fake.listing = DocsChildren(children = listOf(DocsItem(DEVICE, "Laptop", dir = true)))
        val before = provider.queryChildDocuments("devices", null, null as String?).use { it.moveToFirst(); it.getString(0) }
        fake.listing = DocsChildren(children = listOf(DocsItem(DEVICE, "Laptop von Anna", dir = true)))
        val (after, name) = provider.queryChildDocuments("devices", null, null as String?).use {
            it.moveToFirst()
            it.getString(0) to it.getString(it.getColumnIndexOrThrow(Document.COLUMN_DISPLAY_NAME))
        }
        assertEquals(before, after)
        assertEquals("Laptop von Anna", name)
        assertTrue(DocumentIds.isChild(before, "$before/Dokumente/a.txt"))
    }

    @Test
    fun drive_task_android_reader_sees_dirty_entry() {
        // A writer saved without closing; a second descriptor reads through the same core handle
        // table and sees the unpublished content (the engine's dirty entry wins over the remote).
        val writer = ProxyFileCallback(fake, fake.openHandle("gdrive/a.txt"), android.os.HandlerThread("w").apply { start() }) {}
        val data = "neu".toByteArray()
        assertEquals(3, writer.onWrite(0, 3, data))
        val reader = ProxyFileCallback(fake, fake.openHandle("gdrive/a.txt"), android.os.HandlerThread("r").apply { start() }) {}
        val buffer = ByteArray(3)
        assertEquals(3L, reader.onGetSize())
        assertEquals(3, reader.onRead(0, 3, buffer))
        assertEquals("neu", String(buffer))
        assertTrue("Lesen darf nicht veröffentlichen", fake.published.isEmpty())
    }

    @Test
    fun drive_task_android_local_only_flag_only_for_local_vaults() {
        val local = DocumentsCursors.rootFlags(root(VAULT, "Lokal", create = true, localOnly = true))
        val remote = DocumentsCursors.rootFlags(root("vault:${"1".repeat(32)}", "Fern", create = true))
        val conn = DocumentsCursors.rootFlags(root("gdrive", "Google Drive", create = true))
        assertTrue(local and Root.FLAG_LOCAL_ONLY != 0)
        assertEquals(0, remote and Root.FLAG_LOCAL_ONLY)
        assertEquals(0, conn and Root.FLAG_LOCAL_ONLY)
        for (flags in listOf(local, remote, conn)) assertTrue(flags and Root.FLAG_SUPPORTS_IS_CHILD != 0)
    }

    @Test
    fun drive_task_android_provider_init_before_application() {
        // Plain Application: SmartExplorerApp.onCreate never ran, the provider starts the core.
        val provider = provider()
        assertSame(context, started)
        fake.roots = listOf(root("devices", "Smart Explorer – Geräte"))
        assertEquals(listOf("devices"), rootIds(provider))
    }

    @Test
    fun drive_task_android_commit_worker_has_foreground_info() {
        val worker = TestListenableWorkerBuilder<DocsCommitWorker>(context).build()
        val info = runBlocking { worker.getForegroundInfo() }
        assertEquals(DocsCommitWorker.ID_RUNNING, info.notificationId)
        assertEquals(ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC, info.foregroundServiceType)
        assertNotNull(info.notification)
        assertEquals(Notifications.CHANNEL_TRANSFERS, info.notification.channelId)
    }

    private fun root(id: String, title: String, create: Boolean = false, localOnly: Boolean = false) =
        DocsRoot(rootId = id, documentId = id, title = title, create = create, localOnly = localOnly)
}

/** Answers like `docs.*`; one shared content table stands for the core's dirty entries. */
private class FakeBackend : DocsBackend {
    var roots: List<DocsRoot> = emptyList()
    var listing = DocsChildren()
    var openResult: Long? = null
    var failEveryCall = false
    var calls = 0
    val openedModes = mutableListOf<String>()
    val published = mutableListOf<String>()
    private val content = mutableMapOf<String, ByteArray>()
    private val handles = mutableMapOf<Long, String>()

    private fun count() {
        calls++
        if (failEveryCall) throw CoreException("internal", "kein Aufruf erwartet")
    }

    fun openHandle(id: String): Long = (handles.size + 1L).also { handles[it] = id }

    override fun roots(): List<DocsRoot> = roots.also { count() }
    override fun document(id: String): DocsItem = DocsItem(id, id.substringAfterLast('/')).also { count() }
    override fun children(id: String): DocsChildren = listing.also { count() }
    override fun create(parentId: String, name: String, dir: Boolean): DocsItem = DocsItem("$parentId/$name", name, dir).also { count() }
    override fun rename(id: String, name: String): String = "${id.substringBeforeLast('/')}/$name".also { count() }
    override fun delete(id: String) = count()
    override fun open(id: String, mode: String): Long {
        count()
        openedModes += mode
        return openResult ?: openHandle(id)
    }
    override fun read(handle: Long, offset: Long, size: Int, data: ByteArray): Int {
        val bytes = content[handles[handle]] ?: ByteArray(0)
        val count = (bytes.size - offset.toInt()).coerceIn(0, size)
        bytes.copyInto(data, 0, offset.toInt(), offset.toInt() + count)
        return count
    }
    override fun write(handle: Long, offset: Long, size: Int, data: ByteArray): Int {
        val id = handles[handle] ?: return -9
        val old = content[id] ?: ByteArray(0)
        val next = old.copyOf(maxOf(old.size, offset.toInt() + size))
        data.copyInto(next, offset.toInt(), 0, size)
        content[id] = next
        return size
    }
    override fun size(handle: Long): Long = (content[handles[handle]]?.size ?: 0).toLong()
    override fun fsync(handle: Long): Int = 0.also { handles[handle]?.let { published += it } }
    override fun release(handle: Long): Int = 0.also { handles.remove(handle) }
}
