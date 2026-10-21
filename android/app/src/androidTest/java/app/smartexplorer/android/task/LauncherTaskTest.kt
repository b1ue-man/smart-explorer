package app.smartexplorer.android.task

import android.app.ActivityManager
import android.content.ComponentName
import android.content.Intent
import android.content.pm.LauncherApps
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Point
import android.graphics.drawable.Drawable
import android.net.Uri
import android.os.Process
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.os.UserHandle
import android.view.InputDevice
import android.view.MotionEvent
import android.view.ViewConfiguration
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.Until
import androidx.test.uiautomator.UiDevice
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreState
import app.smartexplorer.android.launcher.EnableResult
import app.smartexplorer.android.launcher.HomeResolution
import app.smartexplorer.android.launcher.LauncherState
import app.smartexplorer.android.launcher.LauncherSwitch
import app.smartexplorer.android.launcher.apps.AppCatalog
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.HomeLayout
import app.smartexplorer.android.launcher.model.LayoutJson
import app.smartexplorer.android.launcher.model.LayoutReadResult
import app.smartexplorer.android.launcher.search.LauncherSearch
import app.smartexplorer.android.launcher.search.SearchHit
import app.smartexplorer.android.launcher.search.SearchSource
import app.smartexplorer.android.ui.settings.LauncherSettings
import app.smartexplorer.android.ui.theme.SmartExplorerTheme
import java.io.File
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * DSK-M9, DSK-M10 and DSK-M11 on the device (U35-34, U38-17): the launcher module as a whole. The tests run in the app
 * process (the instrumentation shares it) and look at the launcher process `:launcher` from outside:
 * the component state, `ps`, `/proc/<pid>/maps`, `dumpsys`, the role holders. The device is the suite's,
 * never the user's. `add-role-holder` stands for the person's choice in the system dialog (A32).
 */
@RunWith(AndroidJUnit4::class)
class LauncherTaskTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
    private val device: UiDevice get() = UiDevice.getInstance(instrumentation)
    private val pkg: String get() = appContext.packageName
    private val launcherClass = "app.smartexplorer.android.launcher.LauncherActivity"
    private val launcherComponent: ComponentName get() = ComponentName(pkg, launcherClass)
    private val testHome = ComponentName("app.smartexplorer.android.test", "app.smartexplorer.android.task.TestHomeActivity")
    private val homeRole = "android.app.role.HOME"
    private val emptyLayout = """{"schema":1,"grid":{"columns":4,"rows":5,"iconDp":56},"pages":[{"items":[]}],"dock":[]}"""

    private fun shell(command: String): String {
        val output = instrumentation.uiAutomation.executeShellCommand(command)
        return ParcelFileDescriptor.AutoCloseInputStream(output).bufferedReader().readText()
    }

    private fun homeHolder(): String = shell("cmd role get-role-holders $homeRole").trim()

    private fun waitFor(timeoutMs: Long = 15_000, condition: () -> Boolean): Boolean {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        while (SystemClock.uptimeMillis() < deadline) {
            if (condition()) return true
            SystemClock.sleep(250)
        }
        return condition()
    }

    private fun launcherDir() = LauncherState.dir(appContext)

    /** Processes of this app by name, with their pids. */
    private fun appProcesses(): Map<String, Int> =
        appContext.getSystemService(ActivityManager::class.java).runningAppProcesses.orEmpty().associate { it.processName to it.pid }

    private fun launcherProcessPid(): Int? = appProcesses()["$pkg:launcher"]

    private fun setTestHome(on: Boolean) {
        val state = if (on) PackageManager.COMPONENT_ENABLED_STATE_ENABLED else PackageManager.COMPONENT_ENABLED_STATE_DISABLED
        try {
            appContext.packageManager.setComponentEnabledSetting(testHome, state, PackageManager.DONT_KILL_APP)
        } catch (_: SecurityException) {
            // The test package has its own uid on some builds; the shell may switch its components.
            shell("pm ${if (on) "enable" else "disable"} --user 0 ${testHome.flattenToString()}")
        }
    }

    /** Stands in for the person's choice in the system dialog; `false` when the device refuses the command (A32). */
    private fun addRoleHolder(): Boolean {
        shell("cmd role add-role-holder $homeRole $pkg")
        return waitFor(10_000) { homeHolder().contains(pkg) }
    }

    /** Presses Home and waits until `LauncherActivity` is the top resumed activity. */
    private fun assertLauncherOnTop() {
        device.pressHome()
        assertTrue(
            "LauncherActivity wurde nicht oberstes Fenster",
            waitFor { shell("dumpsys activity activities").lineSequence().any { "ResumedActivity" in it && "launcher.LauncherActivity" in it } },
        )
        assertEquals(pkg, device.currentPackageName)
    }

    private fun resetDevice() {
        runCatching { LauncherSwitch.remove(appContext) }
        shell("cmd appwidget revokebind --package $pkg")
        shell("cmd role remove-role-holder $homeRole $pkg")
        runCatching { setTestHome(false) }
    }

    @Before
    fun before() = resetDevice()

    @After
    fun after() {
        device.pressHome()
        resetDevice()
    }

    @Test
    fun launcher_task_components_disabled_by_default() {
        val info = appContext.packageManager.getActivityInfo(launcherComponent, PackageManager.MATCH_DISABLED_COMPONENTS)
        assertFalse("LauncherActivity must be off before the person switches it on", info.isEnabled)
        assertFalse(LauncherSwitch.isComponentEnabled(appContext))
        assertEquals("$pkg:launcher", info.processName)
        val processes = shell("ps -A")
        assertFalse("process $pkg:launcher exists before the launcher is enabled", processes.lineSequence().any { it.trim().endsWith("$pkg:launcher") })
        assertEquals(null, launcherProcessPid())
    }

    @Test
    fun launcher_task_enable_requests_role() {
        assertEquals(EnableResult.Done, LauncherSwitch.enable(appContext))
        assertTrue(LauncherSwitch.isComponentEnabled(appContext))
        val intent = LauncherSwitch.requestHomeRole(appContext)
        assumeTrue("nicht ausgeführt: das Gerät bietet keine Rollenabfrage", intent != null)
        assertEquals("android.app.role.action.REQUEST_ROLE", intent!!.action)
        assumeTrue("nicht ausgeführt: add-role-holder wurde abgelehnt (A32), Ausgabe: ${homeHolder()}", addRoleHolder())
        assertLauncherOnTop()
        assertTrue(LauncherSwitch.isRoleHeld(appContext))
    }

    @Test
    fun launcher_task_disable_returns_home() {
        launcherDir().mkdirs()
        File(launcherDir(), "layout.json").writeText(emptyLayout)
        assertEquals(EnableResult.Done, LauncherSwitch.enable(appContext))
        assumeTrue("nicht ausgeführt: add-role-holder wurde abgelehnt (A32)", addRoleHolder())
        assertLauncherOnTop()
        assertNotNull(launcherProcessPid())

        val result = LauncherSwitch.disable(appContext)
        assertEquals(null, result.error)
        assertTrue("the role stays with $pkg: ${homeHolder()}", waitFor { !homeHolder().contains(pkg) })
        val widgets = shell("dumpsys appwidget")
        val hostLines = widgets.lineSequence().filter { it.contains("5e57", ignoreCase = true) || it.contains("24151") }.toList()
        assertTrue("host 0x5E57 is still registered: $hostLines", hostLines.isEmpty())
        assertTrue("layout.json was removed by disabling", File(launcherDir(), "layout.json").isFile)
        assertTrue("process $pkg:launcher still runs", waitFor { launcherProcessPid() == null })
        assertFalse(LauncherSwitch.isComponentEnabled(appContext))
        assertFalse(LauncherState.read(appContext).enabled)
    }

    @Test
    fun launcher_task_disable_with_two_launchers_shows_chooser_hint() {
        // The previous launcher is the one the system answers with while only one is installed.
        assertEquals(EnableResult.Done, LauncherSwitch.enable(appContext))
        assertTrue("previous launcher not remembered", LauncherState.read(appContext).previousHome != null)
        setTestHome(true)
        assumeTrue("nicht ausgeführt: add-role-holder wurde abgelehnt (A32)", addRoleHolder())
        val result = LauncherSwitch.disable(appContext)
        assertTrue("two launchers without a default must give the system selection: ${result.home}", result.home == HomeResolution.Chooser)
        assertEquals(HomeResolution.Chooser, LauncherSwitch.resolveHome(appContext))
        compose.setContent { SmartExplorerTheme { LauncherSettings() } }
        compose.onNodeWithText("Wähle deinen Startbildschirm.", substring = true).assertExists()
        compose.onNodeWithText("Startbildschirm wählen").assertExists()
    }

    @Test
    fun launcher_task_remove_deletes_files() {
        launcherDir().mkdirs()
        File(launcherDir(), "layout.json").writeText(emptyLayout)
        File(launcherDir(), "layout.json.corrupt").writeText("x")
        val icons = File(appContext.cacheDir, "launcher-icons").apply { mkdirs() }
        File(icons, "a.png").writeText("x")
        assertEquals(EnableResult.Done, LauncherSwitch.enable(appContext))
        assertTrue(LauncherState.file(appContext).isFile)

        val result = LauncherSwitch.remove(appContext)
        assertEquals(null, result.deleteError)
        assertFalse("launcher directory remains", launcherDir().exists())
        assertFalse("icon cache remains", icons.exists())
        assertFalse(File(appContext.filesDir, "widget").exists())
        assertEquals(
            PackageManager.COMPONENT_ENABLED_STATE_DEFAULT,
            appContext.packageManager.getComponentEnabledSetting(launcherComponent),
        )
        assertFalse(LauncherState.read(appContext).enabled)
        assertFalse("state.json exists after removal", LauncherState.file(appContext).exists())
    }

    @Test
    fun launcher_task_process_has_no_core() {
        assertEquals(EnableResult.Done, LauncherSwitch.enable(appContext))
        assumeTrue("nicht ausgeführt: add-role-holder wurde abgelehnt (A32)", addRoleHolder())
        assertLauncherOnTop()
        val pid = launcherProcessPid()
        assertNotNull("process $pkg:launcher not found", pid)
        val library = "libsmart_explorer_android.so"
        // The positive control: this process has loaded the core, so the maps file proves something.
        assertTrue(File("/proc/self/maps").readText().contains(library))
        val maps = File("/proc/$pid/maps").readText()
        assertFalse("the launcher process loaded $library", maps.contains(library))
    }

    @Test
    fun launcher_task_main_process_unchanged() {
        // The main process runs the seven start steps; the core reports ready as before.
        runBlocking { Api.awaitReady() }
        assertTrue(Core.ready.value is CoreState.Ready)
        assertEquals(pkg, android.app.Application.getProcessName())
        assertFalse("the launcher component must stay off without the person's choice", LauncherSwitch.isComponentEnabled(appContext))
        assertEquals(null, launcherProcessPid())
    }

    @Test
    fun launcher_task_survives_package_replace() {
        val phase = InstrumentationRegistry.getArguments().getString("launcher_replace_phase")
        assumeTrue(
            "nicht ausgeführt: das Ersetzen des Pakets beendet die Instrumentierung; der Wirtslauf ruft diesen Test mit " +
                "'-e launcher_replace_phase before', dann 'adb install -r' derselben APK, dann '-e launcher_replace_phase after' auf",
            phase == "before" || phase == "after",
        )
        if (phase == "before") {
            assertEquals(EnableResult.Done, LauncherSwitch.enable(appContext))
            assertTrue("add-role-holder wurde abgelehnt (A32)", addRoleHolder())
            assertLauncherOnTop()
            File(appContext.filesDir, "launcher-replace-marker").writeText("before")
        } else {
            assertTrue("marker of the first phase is missing", File(appContext.filesDir, "launcher-replace-marker").isFile)
            assertTrue("component state was lost by the package replacement (A30)", LauncherSwitch.isComponentEnabled(appContext))
            assertTrue("the role left $pkg: ${homeHolder()}", homeHolder().contains(pkg))
            assertLauncherOnTop()
            File(appContext.filesDir, "launcher-replace-marker").delete()
        }
    }

    @Test
    fun launcher_task_nav_token_from_main_only() {
        val prefs = File(appContext.dataDir, "shared_prefs/nav.xml")
        assertEquals(EnableResult.Done, LauncherSwitch.enable(appContext))
        prefs.delete()
        shell("am start -n $pkg/.launcher.LauncherActivity")
        assertTrue("process $pkg:launcher did not start", waitFor { launcherProcessPid() != null })
        SystemClock.sleep(3_000)
        assertFalse("the launcher process created ${prefs.path} (it called AppNav.token)", prefs.exists())
    }

    // ---- DSK-M10: the basic functions ------------------------------------------------------------------------

    /** Switches the launcher on, makes it the default home app and shows it; skips the test when the device refuses (A32). */
    private fun startLauncher() {
        assertEquals(EnableResult.Done, LauncherSwitch.enable(appContext))
        assumeTrue("nicht ausgeführt: add-role-holder wurde abgelehnt (A32)", addRoleHolder())
        assertLauncherOnTop()
    }

    private fun readLayout(): HomeLayout? {
        val file = File(launcherDir(), "layout.json")
        if (!file.isFile) return null
        return (LayoutJson.parse(file.readText()) as? LayoutReadResult.Loaded)?.layout
    }

    /** The apps of page 1 of the first arrangement, with their names. */
    private fun firstRunApps(): List<Pair<HomeItem, String>> {
        val catalog = AppCatalog.get(appContext)
        catalog.reload()
        assertTrue("the launcher wrote no layout.json", waitFor { readLayout() != null })
        return readLayout()!!.pages[0].filter { it.isApp }.mapNotNull { item ->
            catalog.find(item.component!!, item.userSerial)?.let { item to it.label }
        }
    }

    /** Touch down, wait the long-press time, move in steps to [to], rest [restMs] there, lift: what a person's drag is. */
    private fun dragAndHold(from: Point, to: Point, restMs: Long) {
        val automation = instrumentation.uiAutomation
        val downTime = SystemClock.uptimeMillis()
        fun send(action: Int, x: Float, y: Float) {
            val event = MotionEvent.obtain(downTime, SystemClock.uptimeMillis(), action, x, y, 0)
            event.source = InputDevice.SOURCE_TOUCHSCREEN
            automation.injectInputEvent(event, true)
            event.recycle()
        }
        send(MotionEvent.ACTION_DOWN, from.x.toFloat(), from.y.toFloat())
        SystemClock.sleep(ViewConfiguration.getLongPressTimeout() + 300L)
        val steps = 12
        for (step in 1..steps) {
            send(MotionEvent.ACTION_MOVE, from.x + (to.x - from.x) * step / steps.toFloat(), from.y + (to.y - from.y) * step / steps.toFloat())
            SystemClock.sleep(30)
        }
        SystemClock.sleep(restMs)
        send(MotionEvent.ACTION_UP, to.x.toFloat(), to.y.toFloat())
    }

    @Test
    fun launcher_task_app_list_matches_launcherapps() {
        startLauncher()
        val launcherApps = appContext.getSystemService(LauncherApps::class.java)
        val expected = launcherApps.profiles.flatMap { user -> launcherApps.getActivityList(null, user).map { it.label.toString() } }.toSet()
        val catalog = AppCatalog.get(appContext)
        catalog.reload()
        val shown = catalog.visible.map { it.label }.toSet()
        // The private space (API 35) is left out until U38; everything else is in the list.
        assertTrue("apps missing in the list: ${expected - shown}", shown.containsAll(expected.filter { name -> catalog.apps.value.none { it.label == name && it.kind == app.smartexplorer.android.launcher.apps.ProfileKind.Private } }))
        // The list on the screen: open it and see the search field and one of the names.
        val button = device.wait(Until.findObject(By.desc("Alle Apps")), 10_000)
        assertNotNull("button „Alle Apps“ not found", button)
        button.click()
        assertTrue("app list did not open", device.wait(Until.hasObject(By.text("Apps suchen")), 10_000))
        assertTrue("no app name visible in the list", catalog.visible.take(40).any { device.hasObject(By.text(it.label)) })
    }

    @Test
    fun launcher_task_search_finds_app_and_device() {
        val catalog = AppCatalog.get(appContext)
        catalog.reload()
        val app = catalog.visible.first()
        val word = app.label.split(Regex("[^\\p{L}\\p{N}]+")).first { it.isNotEmpty() }
        val source = SearchSource { tokens ->
            if (tokens.all { LauncherSearch.normalize("Test-Gerät").contains(it) }) {
                listOf(SearchHit("Geräte und Räume", "Test-Gerät", "Online") {})
            } else {
                emptyList()
            }
        }
        LauncherSearch.sources.add(source)
        try {
            val apps = LauncherSearch.filter(catalog.visible, word, { it.label })
            assertTrue("the app \"${app.label}\" is not found by \"$word\"", apps.any { it.key == app.key })
            val hits = LauncherSearch.additional("test ger")
            assertEquals(listOf("Test-Gerät"), hits.map { it.title })
            assertEquals("Geräte und Räume", hits.single().source)
            assertTrue(LauncherSearch.additional("zzzz-nichts").isEmpty())
        } finally {
            LauncherSearch.sources.remove(source)
        }
    }

    @Test
    fun launcher_task_folder_by_drag() {
        startLauncher()
        val apps = firstRunApps()
        assumeTrue("nicht ausgeführt: weniger als zwei Apps auf der ersten Seite", apps.size >= 2)
        val (first, second) = apps[0] to apps[1]
        val source = device.wait(Until.findObject(By.desc(first.second)), 10_000)
        val target = device.wait(Until.findObject(By.desc(second.second)), 10_000)
        assertNotNull(source)
        assertNotNull(target)
        dragAndHold(source.visibleCenter, target.visibleCenter, restMs = 900)
        assertTrue(
            "no folder with both apps in layout.json",
            waitFor {
                readLayout()?.pages?.flatten()?.any { it.isFolder && it.children.map { child -> child.id }.toSet() == setOf(first.first.id, second.first.id) } == true
            },
        )
    }

    @Test
    fun launcher_task_uninstall_intent_resolves() {
        // The permission REQUEST_DELETE_PACKAGES lets this app start the system uninstaller (DN16); the dialog is cancelled.
        val target = "app.smartexplorer.android.test"
        val intent = Intent(Intent.ACTION_DELETE, Uri.parse("package:$target"))
        val resolved = appContext.packageManager.resolveActivity(intent, android.content.pm.PackageManager.MATCH_DEFAULT_ONLY)
        assertNotNull("no activity answers ACTION_DELETE", resolved)
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        appContext.startActivity(intent)
        val packageName = resolved!!.activityInfo.packageName
        assertTrue("the uninstall dialog did not open", device.wait(Until.hasObject(By.pkg(packageName)), 15_000))
        device.pressBack()
        assertTrue("the test package is gone", appContext.packageManager.getInstalledPackages(0).any { it.packageName == target })
    }

    @Test
    fun launcher_task_app_info_opens() {
        startLauncher()
        val own = firstRunApps().firstOrNull { it.first.component!!.startsWith("$pkg/") }
        assumeTrue("nicht ausgeführt: die App steht nicht auf der ersten Seite", own != null)
        val tile = device.wait(Until.findObject(By.desc(own!!.second)), 10_000)
        assertNotNull(tile)
        // A long press that stays in place opens the menu.
        dragAndHold(tile.visibleCenter, tile.visibleCenter, restMs = 0)
        val row = device.wait(Until.findObject(By.text("App-Info")), 10_000)
        assertNotNull("menu without „App-Info“", row)
        row.click()
        assertTrue("the app info did not open", waitFor { device.currentPackageName != pkg })
        device.pressBack()
    }

    @Test
    fun launcher_task_home_key_resets() {
        startLauncher()
        val button = device.wait(Until.findObject(By.desc("Alle Apps")), 10_000)
        assertNotNull(button)
        button.click()
        assertTrue(device.wait(Until.hasObject(By.text("Apps suchen")), 10_000))
        device.pressHome()
        assertTrue("the app list stays open after the Home key", device.wait(Until.gone(By.text("Apps suchen")), 10_000))
        assertTrue(device.wait(Until.hasObject(By.desc("Alle Apps")), 10_000))
    }

    @Test
    fun launcher_task_package_removed_clears_items() {
        val apk = InstrumentationRegistry.getArguments().getString("launcher_helper_apk")
        val helper = InstrumentationRegistry.getArguments().getString("launcher_helper_package")
        assumeTrue(
            "nicht ausgeführt: braucht '-e launcher_helper_apk <Pfad auf dem Gerät>' und '-e launcher_helper_package <Paket>' " +
                "(eine kleine App mit LAUNCHER-Aktivität); ohne sie lässt sich nichts installieren und wieder entfernen",
            !apk.isNullOrBlank() && !helper.isNullOrBlank(),
        )
        shell("pm uninstall $helper")
        assertTrue(shell("pm install -r $apk").contains("Success"))
        try {
            val catalog = AppCatalog.get(appContext)
            catalog.reload()
            val entry = catalog.visible.firstOrNull { it.packageName == helper }
            assertNotNull("the helper app has no launcher activity", entry)
            // Put the app on page 1 while the launcher is not running, then start it.
            launcherDir().mkdirs()
            val onHome = HomeItem(
                id = "ihelper",
                kind = "app",
                attrs = kotlinx.serialization.json.JsonObject(
                    mapOf(
                        "component" to kotlinx.serialization.json.JsonPrimitive(entry!!.componentString),
                        "user" to kotlinx.serialization.json.JsonPrimitive(entry.userSerial),
                    ),
                ),
            )
            val layout = HomeLayout(
                grid = app.smartexplorer.android.launcher.model.GridSpec(4, 5, 56),
                pages = listOf(listOf(onHome)),
                dock = emptyList(),
            )
            File(launcherDir(), "layout.json").writeText(LayoutJson.encode(layout))
            startLauncher()
            assertTrue("the helper app is not on the home screen", device.wait(Until.hasObject(By.desc(entry.label)), 10_000))
            shell("pm uninstall $helper")
            assertTrue("the item stays after the app is removed", device.wait(Until.gone(By.desc(entry.label)), 15_000))
            assertTrue(
                "layout.json still names the removed app",
                waitFor { readLayout()?.pages?.flatten()?.none { it.packageName == helper } == true },
            )
        } finally {
            shell("pm uninstall $helper")
        }
    }

    @Test
    fun launcher_task_work_profile_badged() {
        val created = shell("pm create-user --profileOf 0 --managed Arbeit")
        val id = Regex("""id\s+(\d+)""").find(created)?.groupValues?.get(1)?.toIntOrNull()
        assumeTrue("nicht ausgeführt: Arbeitsprofil nicht anlegbar (DN17): $created", id != null)
        try {
            val work: UserHandle = UserHandle.of(id!!)
            val pm = appContext.packageManager
            val icon = pm.getApplicationIcon(pkg)
            val plain = render(icon.constantState!!.newDrawable().mutate())
            val badged = render(pm.getUserBadgedIcon(icon.constantState!!.newDrawable().mutate(), work))
            assertFalse("the work badge changed nothing", plain.sameAs(badged))
            // The own profile gets no badge.
            val own = render(pm.getUserBadgedIcon(icon.constantState!!.newDrawable().mutate(), Process.myUserHandle()))
            assertTrue(plain.sameAs(own))
        } finally {
            shell("pm remove-user $id")
        }
    }

    // ---- DSK-M11: widgets and pin requests (U38-17) ------------------------------------------------------

    private val testPackage = "app.smartexplorer.android.test"
    private val pinRequester = "$testPackage/app.smartexplorer.android.task.TestPinRequesterActivity"

    /** The one-time approval „Immer erlauben“ for this app (DN18), so binding needs no dialog. */
    private fun grantBind() = shell("cmd appwidget grantbind --package $pkg")

    /** A report line file of the test widget, read from the test package's data with `run-as`; empty when there is none. */
    private fun report(name: String): List<List<String>> {
        val text = shell("run-as $testPackage cat files/task-report/$name")
        if ("No such file" in text || text.startsWith("run-as:")) return emptyList()
        return text.lines().filter { it.isNotBlank() }.map { it.split('\t') }
    }

    private fun clearReports() {
        shell("run-as $testPackage rm -rf files/task-report")
    }

    private fun testWidgets(): List<HomeItem> =
        readLayout()?.pages?.flatten().orEmpty().filter { it.isWidget && it.provider?.startsWith("$testPackage/") == true }

    /** Writes an empty first page, so the middle of the screen is an empty cell, then shows the launcher. */
    private fun startEmptyLauncher() {
        launcherDir().mkdirs()
        File(launcherDir(), "layout.json").writeText(emptyLayout)
        startLauncher()
        assertTrue("the empty page is not shown", device.wait(Until.hasObject(By.desc("Seite 1 von 1")), 10_000))
    }

    /** Long press on the empty middle of the page: the settings sheet, then „Widgets“, the search for the test widget. */
    private fun openPickerForTestWidget() {
        device.swipe(device.displayWidth / 2, device.displayHeight / 2, device.displayWidth / 2, device.displayHeight / 2, 250)
        val sheet = device.wait(Until.findObject(By.text("Widgets")), 10_000)
        assertNotNull("the settings sheet has no entry „Widgets“", sheet)
        sheet.click()
        val field = device.wait(Until.findObject(By.text("Widgets suchen")), 10_000)
        assertNotNull("the sheet „Widgets“ did not open", field)
        field.text = "Test-Widget"
    }

    /** Adds the test widget through the sheet and waits until the configuration ran and the widget is in the layout. */
    private fun addTestWidget() {
        grantBind()
        clearReports()
        openPickerForTestWidget()
        val add = device.wait(Until.findObject(By.text("Hinzufügen")), 15_000)
        assertNotNull("no „Hinzufügen“ for the test widget (group of the test APK)", add)
        add.click()
        assertTrue("the configuration activity did not run", waitFor(20_000) { report("widget-config.tsv").isNotEmpty() })
        assertTrue("the widget is not in layout.json", waitFor(20_000) { testWidgets().isNotEmpty() })
    }

    @Test
    fun launcher_task_widget_bind_config_resize() {
        startEmptyLauncher()
        addTestWidget()
        val placed = testWidgets().single()
        assertEquals("the widget is on page 1", 1, readLayout()!!.pages.indexOfFirst { page -> page.any { it.id == placed.id } } + 1)
        assertTrue("the widget was set up by its configuration", placed.configured)
        assertNotNull("the widget id is missing", placed.widgetId)

        // The size messages reached the provider, in dp of the span's area.
        assertTrue("no size message reached the provider", waitFor(20_000) { report("widget-options.tsv").isNotEmpty() })
        val density = appContext.resources.displayMetrics.density
        val screenWidthDp = appContext.resources.displayMetrics.widthPixels / density
        val screenHeightDp = appContext.resources.displayMetrics.heightPixels / density
        fun checkSizes(line: List<String>, span: app.smartexplorer.android.launcher.model.Span) {
            val minWidth = line[2].toInt()
            val minHeight = line[3].toInt()
            val maxWidth = line[4].toInt()
            val maxHeight = line[5].toInt()
            assertTrue("OPTION_APPWIDGET_MIN_WIDTH $minWidth outside the span ${span.w}", minWidth in (span.w * 72 - 1)..(span.w * maxOf(screenWidthDp, screenHeightDp)).toInt() + 1)
            assertTrue("OPTION_APPWIDGET_MAX_HEIGHT $maxHeight outside the span ${span.h}", maxHeight in (span.h * 92 - 1)..(span.h * maxOf(screenWidthDp, screenHeightDp)).toInt() + 1)
            assertTrue("sizes $minWidth..$maxWidth x $minHeight..$maxHeight", minWidth > 0 && minHeight > 0 && maxWidth > 0 && maxHeight > 0)
        }
        val first = report("widget-options.tsv").last()
        checkSizes(first, placed.span)

        // Pull the right handle by one cell (menu „Größe ändern“); the widget gets wider and the provider is told again.
        val before = report("widget-options.tsv").size
        val tile = device.wait(Until.findObject(By.desc("se Test-Widget")), 10_000)
        assertNotNull("the widget cell has no description", tile)
        dragAndHold(tile.visibleCenter, tile.visibleCenter, restMs = 0)
        val resize = device.wait(Until.findObject(By.text("Größe ändern")), 10_000)
        assertNotNull("menu without „Größe ändern“", resize)
        resize.click()
        val handle = device.wait(Until.findObject(By.desc("Griff rechts")), 10_000)
        assertNotNull("no handle on the right side", handle)
        val columns = readLayout()!!.grid.columns.coerceAtLeast(1)
        val cellPx = device.displayWidth / columns
        val from = handle.visibleCenter
        handle.drag(Point(from.x + cellPx, from.y))
        assertTrue(
            "the span did not grow by one column",
            waitFor { testWidgets().singleOrNull()?.span?.w == placed.span.w + 1 },
        )
        assertTrue("the provider was not told the new size", waitFor(20_000) { report("widget-options.tsv").size > before })
        val last = report("widget-options.tsv").last()
        checkSizes(last, testWidgets().single().span)
        assertTrue("minimum width did not grow: ${first[2]} -> ${last[2]}", last[2].toInt() > first[2].toInt())
    }

    @Test
    fun launcher_task_widget_reconfigure() {
        startEmptyLauncher()
        addTestWidget()
        val runs = report("widget-config.tsv").size
        val tile = device.wait(Until.findObject(By.desc("se Test-Widget")), 10_000)
        assertNotNull("the widget cell has no description", tile)
        dragAndHold(tile.visibleCenter, tile.visibleCenter, restMs = 0)
        val row = device.wait(Until.findObject(By.text("Neu einrichten")), 10_000)
        assertNotNull("menu without „Neu einrichten“ for a reconfigurable widget", row)
        row.click()
        assertTrue("the configuration activity did not run again", waitFor(20_000) { report("widget-config.tsv").size > runs })
        assertTrue("the widget left the layout", testWidgets().isNotEmpty())
    }

    @Test
    fun launcher_task_widgets_restored_after_reenable() {
        startEmptyLauncher()
        addTestWidget()
        val before = testWidgets().single()
        val result = LauncherSwitch.disable(appContext)
        assertEquals(null, result.error)
        assertTrue("process $pkg:launcher still runs", waitFor { launcherProcessPid() == null })
        assertEquals("the item stays while the launcher is off", before.id, testWidgets().single().id)

        startLauncher()
        // No bind dialog: the approval stands; the provider has a configuration, so the cell asks to set it up.
        assertTrue("the widget cell is not back", device.wait(Until.hasObject(By.text("Einrichten")), 20_000))
        assertEquals("a dialog covers the launcher", pkg, device.currentPackageName)
        val after = testWidgets().single()
        assertEquals(before.id, after.id)
        assertTrue("the widget still has its old id ${before.widgetId}", waitFor { testWidgets().single().widgetId != before.widgetId })
        assertFalse("configured must be false after the rebind", testWidgets().single().configured)
    }

    private fun startRequester(kind: String) {
        shell("am start -n $pinRequester --es kind $kind")
    }

    @Test
    fun launcher_task_pin_shortcut_request() {
        startEmptyLauncher()
        startRequester("shortcut")
        assertTrue("no dialog „Verknüpfung hinzufügen“", device.wait(Until.hasObject(By.text("Verknüpfung hinzufügen")), 15_000))
        assertNotNull(device.findObject(By.text("se Test-Verknüpfung")))
        device.findObject(By.text("Hinzufügen")).click()
        assertTrue(
            "the shortcut is not on page 1",
            waitFor {
                readLayout()?.pages?.firstOrNull()?.any { it.isShortcut && it.attr("shortcutId") == "se-test-pin" && it.component?.startsWith("$testPackage/") == true } == true
            },
        )
    }

    @Test
    fun launcher_task_pin_widget_request() {
        grantBind()
        clearReports()
        startEmptyLauncher()
        startRequester("widget")
        assertTrue("no dialog „Widget hinzufügen“", device.wait(Until.hasObject(By.text("Widget hinzufügen")), 15_000))
        device.findObject(By.text("Hinzufügen")).click()
        assertTrue("the configuration activity did not run", waitFor(20_000) { report("widget-config.tsv").isNotEmpty() })
        assertTrue("the pinned widget is not on page 1", waitFor(20_000) { readLayout()?.pages?.firstOrNull()?.any { it in testWidgets() } == true })
    }

    @Test
    fun launcher_task_pin_activity_rejects_forged_intent() {
        assertEquals(EnableResult.Done, LauncherSwitch.enable(appContext))
        shell("am start -n $pkg/.launcher.pin.PinRequestActivity -a android.content.pm.action.CONFIRM_PIN_SHORTCUT")
        assertTrue("the activity stays after a forged intent", waitFor(15_000) { !shell("dumpsys activity activities").contains("PinRequestActivity") })
        assertFalse(device.hasObject(By.text("Verknüpfung hinzufügen")))
        assertFalse(device.hasObject(By.text("Widget hinzufügen")))
        shell("am start -n $pkg/.launcher.pin.PinRequestActivity -a android.content.pm.action.CONFIRM_PIN_APPWIDGET")
        assertTrue("the activity stays after a forged widget intent", waitFor(15_000) { !shell("dumpsys activity activities").contains("PinRequestActivity") })
        assertFalse(device.hasObject(By.text("Widget hinzufügen")))
    }

    private fun render(drawable: Drawable, size: Int = 192): Bitmap {
        val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        drawable.setBounds(0, 0, size, size)
        drawable.draw(Canvas(bitmap))
        return bitmap
    }
}
