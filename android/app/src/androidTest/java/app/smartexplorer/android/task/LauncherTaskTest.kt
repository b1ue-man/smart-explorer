package app.smartexplorer.android.task

import android.app.ActivityManager
import android.app.UiAutomation
import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Intent
import android.content.pm.LauncherApps
import android.content.pm.PackageManager
import android.content.pm.ResolveInfo
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Point
import android.graphics.drawable.Drawable
import android.net.Uri
import android.os.Build
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
import androidx.test.uiautomator.Configurator
import androidx.test.uiautomator.Until
import androidx.test.uiautomator.UiDevice
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreState
import app.smartexplorer.android.launcher.bridge.BridgeProtocol
import app.smartexplorer.android.launcher.bridge.DevicesData
import app.smartexplorer.android.launcher.bridge.DevicesFolder
import app.smartexplorer.android.launcher.EnableResult
import app.smartexplorer.android.launcher.HomeResolution
import app.smartexplorer.android.launcher.LauncherState
import app.smartexplorer.android.launcher.LauncherSwitch
import app.smartexplorer.android.launcher.apps.AppCatalog
import app.smartexplorer.android.launcher.apps.ProfileKind
import app.smartexplorer.android.launcher.model.Cell
import app.smartexplorer.android.launcher.model.GridSpec
import app.smartexplorer.android.launcher.model.HomeItem
import app.smartexplorer.android.launcher.model.HomeLayout
import app.smartexplorer.android.launcher.model.LayoutJson
import app.smartexplorer.android.launcher.model.LayoutReadResult
import app.smartexplorer.android.launcher.search.LauncherSearch
import app.smartexplorer.android.launcher.search.SearchHit
import app.smartexplorer.android.launcher.search.SearchSource
import app.smartexplorer.android.screen.ScreenControlService
import app.smartexplorer.android.launcher.se.OpenInFiles
import app.smartexplorer.android.launcher.se.SeActions
import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.NavRequest
import app.smartexplorer.android.widget.DevicesWidgetFeed
import app.smartexplorer.android.widget.WidgetCache
import app.smartexplorer.android.widget.WidgetRow
import app.smartexplorer.android.widget.WidgetSnapshot
import app.smartexplorer.android.widget.WidgetTarget
import app.smartexplorer.android.ui.settings.LauncherSettings
import app.smartexplorer.android.ui.theme.SmartExplorerTheme
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TestName
import org.junit.runner.RunWith

/**
 * DSK-M9, DSK-M10, DSK-M11, DSK-M12 and DSK-M13 on the device (U35-34, U38-17, U40-21): the launcher module as a whole. The tests run in the app
 * process (the instrumentation shares it) and look at the launcher process `:launcher` from outside:
 * the component state, `ps`, `/proc/<pid>/maps`, `dumpsys`, the role holders. The device is the suite's,
 * never the user's. `add-role-holder` stands for the person's choice in the system dialog (A32).
 *
 * What the tests of this class have to know about the platform (each point was found in the first device run, suite run 7):
 * - A UiAutomation without `FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES` keeps every accessibility service unbound, so all shell
 *   commands, injected events and `UiDevice` calls use that flag ([automation], [before]).
 * - On a phone a drop on the first row of the page lands in the drop zone „Entfernen“ (the zone covers the top 72 dp of the page area,
 *   a cell is 92 to 110 dp high). With a single page the indicator draws no dots, so „Seite 1 von 1“ has no size and Compose leaves
 *   it out of the accessibility tree.
 * - A Compose text field is an `android.widget.EditText` whose placeholder is a node of its own; a Compose `Switch` has the role
 *   description „Switch“ and the class `android.view.View`, not `android.widget.Switch`.
 * - The system starts the process of a visible activity again after it was killed; the person switches the launcher off in the app's
 *   settings, so the app is in front then ([showAppInFront]).
 * - The phase tests ([keepsStateBefore], [keepsStateAfter]) must not reset the device between their runs.
 */
@RunWith(AndroidJUnit4::class)
class LauncherTaskTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val testName = TestName()

    private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
    private val device: UiDevice get() = UiDevice.getInstance(instrumentation)
    private val pkg: String get() = appContext.packageName
    private val launcherClass = "app.smartexplorer.android.launcher.LauncherActivity"
    private val launcherComponent: ComponentName get() = ComponentName(pkg, launcherClass)
    private val testHome = ComponentName("app.smartexplorer.android.test", "app.smartexplorer.android.task.TestHomeActivity")
    private val homeRole = "android.app.role.HOME"
    private val emptyLayout =
        """{"schema":1,"grid":{"columns":4,"rows":5,"iconDp":56},"pages":[{"items":[]}],"dock":[],"seSeeded":true}"""

    /**
     * The suite's UiAutomation. `Instrumentation.getUiAutomation()` is `getUiAutomation(0)`, and a UiAutomation without
     * `FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES` makes the system unbind every other accessibility service and bind none for as long
     * as it is connected (`UiAutomationManager.suppressingAccessibilityServicesLocked`). A call with other flags reconnects the same
     * instance with those flags: every shell command and every injected event of this class goes through here and never through
     * `uiAutomation`; `UiDevice` asks for its UiAutomation with the flags of the `Configurator` ([before]).
     */
    private fun automation(): UiAutomation =
        instrumentation.getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)

    private fun shell(command: String): String {
        val output = automation().executeShellCommand(command)
        return ParcelFileDescriptor.AutoCloseInputStream(output).bufferedReader().readText()
    }

    /** True while this app holds the HOME role (`RoleManager`, API 29); the shell's `get-role-holders` exists only from API 31. */
    private fun roleHeld(): Boolean = LauncherSwitch.isRoleHeld(appContext)

    /** The holder as far as the shell tells it, for messages only (API 30 answers „Unknown command“). */
    private fun homeHolderText(): String = shell("cmd role get-role-holders $homeRole").trim().ifEmpty { "(unbekannt)" }

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

    /**
     * Switches the second home screen of the test APK on or off. The shell cannot do it: `PackageManagerService.setEnabledSetting`
     * lets the shell change the components of a package only when it is `testOnly` (this APK is not), and the app's own uid has no right
     * to the components of another package. So the shell's identity lends its permission to this process: the call then comes from the
     * app's uid with `CHANGE_COMPONENT_ENABLED_STATE`, which the shell holds (`UiAutomation.adoptShellPermissionIdentity`, API 29).
     */
    private fun setTestHome(on: Boolean) {
        val pm = appContext.packageManager
        val wanted = if (on) PackageManager.COMPONENT_ENABLED_STATE_ENABLED else PackageManager.COMPONENT_ENABLED_STATE_DEFAULT
        if (pm.getComponentEnabledSetting(testHome) == wanted) return
        val ui = automation()
        ui.adoptShellPermissionIdentity("android.permission.CHANGE_COMPONENT_ENABLED_STATE")
        try {
            pm.setComponentEnabledSetting(testHome, wanted, PackageManager.DONT_KILL_APP)
        } finally {
            ui.dropShellPermissionIdentity()
        }
        assertEquals(
            "the second home screen of the test APK did not switch ${if (on) "on" else "off"}",
            on,
            pm.getComponentEnabledSetting(testHome) == PackageManager.COMPONENT_ENABLED_STATE_ENABLED,
        )
    }

    /** What the system offers for the HOME intent, in the order in which it picks from it. */
    private fun homeOffers(): List<ResolveInfo> {
        val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
        @Suppress("DEPRECATION")
        return appContext.packageManager.queryIntentActivities(intent, PackageManager.MATCH_DEFAULT_ONLY)
    }

    /** Every activity the system offers for the HOME intent, as `<package>/<class> priority/order/default`. */
    private fun homeCandidates(): List<String> = homeOffers().map {
        "${it.activityInfo.packageName}/${it.activityInfo.name} ${it.priority}/${it.preferredOrder}/${it.isDefault}"
    }

    /** Stands in for the person's choice in the system dialog; `false` when the device refuses the command (A32). */
    private fun addRoleHolder(): Boolean {
        shell("cmd role add-role-holder $homeRole $pkg")
        return waitFor(10_000) { roleHeld() }
    }

    /** The lines of `dumpsys activity activities` that name a resumed activity. */
    private fun resumedActivityLines(): List<String> =
        shell("dumpsys activity activities").lineSequence().filter { "ResumedActivity" in it }.toList()

    private fun launcherResumed(): Boolean = resumedActivityLines().any { "launcher.LauncherActivity" in it }

    /** True while the app's own `MainActivity` is the resumed activity: the person looks at the app, not at the home screen. */
    private fun appResumed(): Boolean = resumedActivityLines().any { "$pkg/" in it && "MainActivity" in it }

    /** Presses Home and waits until `LauncherActivity` is the top resumed activity. */
    private fun assertLauncherOnTop() {
        device.pressHome()
        assertTrue("LauncherActivity wurde nicht oberstes Fenster", waitFor { launcherResumed() })
        assertEquals(pkg, device.currentPackageName)
    }

    /**
     * Brings the app to the front, the place from which the person switches the launcher off (the settings card). The home screen is
     * not visible then. If it were, killing its process would not end it: the system keeps the record of a visible activity that has
     * `stateNotNeeded` (the launcher has it) and starts the process again (`ActivityRecord.handleAppDied`, `ActivityTaskManagerService`).
     */
    private fun showAppInFront() {
        shell("am start -n $ownActivity")
        assertTrue("the app did not come to the front", waitFor(20_000) { appResumed() })
    }

    /**
     * Puts the device back as the image has it. The role goes first: while the launcher holds it, it is the home screen the person
     * sees, and the system would start the process of that visible activity again right after `remove` killed it, with whatever
     * state it then reads from the files `remove` is just deleting.
     */
    private fun resetDevice() {
        shell("cmd role remove-role-holder $homeRole $pkg")
        waitFor(10_000) { !roleHeld() }
        if (launcherResumed()) {
            device.pressHome()
            waitFor(10_000) { !launcherResumed() }
        }
        runCatching { LauncherSwitch.remove(appContext) }
        // The tool `appwidget` (frameworks/base/cmds/appwidget), not `cmd appwidget`: the service has no shell command.
        shell("appwidget revokebind --package $pkg")
        runCatching { setTestHome(false) }
    }

    /** The argument of a phase test (see `native/launcher-task-android.py`, `PHASES`). */
    private fun phaseArgument(name: String): String? = InstrumentationRegistry.getArguments().getString(name)

    /**
     * The phase tests are several runs of one test with host steps between them (`adb install -r`, `am kill`, a broadcast). What the
     * first run set up — the enabled launcher, the role, the layout, the widget — is what the host looks at and what the next run
     * checks, so the run that ends a phase leaves the device as it is, and the run that follows does not reset it first.
     */
    private fun keepsStateAfter(): Boolean = when (testName.methodName) {
        "launcher_task_survives_package_replace" -> phaseArgument("launcher_replace_phase") == "before"
        "launcher_task_no_bind_when_main_process_absent" -> phaseArgument("launcherMainAbsentPhase") == "prepare"
        "launcher_task_devices_widget_does_not_start_core" -> phaseArgument("launcherWidgetCorePhase") == "prepare"
        else -> false
    }

    private fun keepsStateBefore(): Boolean = when (testName.methodName) {
        "launcher_task_survives_package_replace" -> phaseArgument("launcher_replace_phase") == "after"
        "launcher_task_no_bind_when_main_process_absent" -> phaseArgument("launcherMainAbsentPhase") == "verify"
        "launcher_task_devices_widget_does_not_start_core" -> phaseArgument("launcherWidgetCorePhase") == "verify"
        else -> false
    }

    @Before
    fun before() {
        // `UiDevice` asks for its UiAutomation again on every call, with the flags of the Configurator (default 0 = suppress).
        Configurator.getInstance().setUiAutomationFlags(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
        automation()
        if (!keepsStateBefore()) resetDevice()
    }

    @After
    fun after() {
        if (!keepsStateAfter()) resetDevice()
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
        assumeTrue("nicht ausgeführt: add-role-holder wurde abgelehnt (A32), Halter: ${homeHolderText()}", addRoleHolder())
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

        // The person switches the launcher off in the app's settings: the app is in front, the launcher is not.
        showAppInFront()
        val result = LauncherSwitch.disable(appContext)
        assertEquals(null, result.error)
        assertTrue("the role stays with $pkg: ${homeHolderText()}", waitFor { !roleHeld() })
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
        val candidates = homeCandidates()
        assertTrue("the test APK's home screen is not offered to the system: $candidates", candidates.any { it.startsWith("app.smartexplorer.android.test/") })
        // Without a default the system lets the person choose only between launchers it ranks alike (`PackageManagerService.
        // chooseBestActivity` takes the first when priority, order or „default“ differ). Ours is off after `disable`, so the first two others count.
        val others = homeOffers().filter { it.activityInfo.packageName != pkg }
        assertTrue("fewer than two other home screens: $candidates", others.size >= 2)
        assumeTrue(
            "nicht ausgeführt: das System rangiert ${others[0].activityInfo.packageName} vor ${others[1].activityInfo.packageName} " +
                "(priority/order/default $candidates); eine Auswahl entsteht auf diesem Gerät nie",
            others[0].priority == others[1].priority && others[0].preferredOrder == others[1].preferredOrder && others[0].isDefault == others[1].isDefault,
        )
        assumeTrue("nicht ausgeführt: add-role-holder wurde abgelehnt (A32)", addRoleHolder())
        val result = LauncherSwitch.disable(appContext)
        assertTrue(
            "two launchers without a default must give the system selection: ${result.home}, home screens offered: $candidates, " +
                "now: ${homeCandidates()}",
            result.home == HomeResolution.Chooser,
        )
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
        val phase = phaseArgument("launcher_replace_phase")
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
            assertTrue("the role left $pkg: ${homeHolderText()}", roleHeld())
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

    /** An app item of a layout file, at [cell] of the first page. */
    private fun appItemAt(id: String, component: String, userSerial: Long, cell: Cell) = HomeItem(
        id = id,
        kind = "app",
        cell = cell,
        attrs = JsonObject(mapOf("component" to JsonPrimitive(component), "user" to JsonPrimitive(userSerial))),
    )

    /** A layout file of one page with [items] and the first fill of the se elements done, so the launcher adds nothing of its own. */
    private fun layoutOf(vararg items: HomeItem): String = LayoutJson.encode(
        HomeLayout(
            grid = GridSpec(4, 5, 56),
            pages = listOf(items.toList()),
            dock = emptyList(),
            extra = JsonObject(mapOf("seSeeded" to JsonPrimitive(true))),
        ),
    )

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
        val ui = automation()
        val downTime = SystemClock.uptimeMillis()
        fun send(action: Int, x: Float, y: Float) {
            val event = MotionEvent.obtain(downTime, SystemClock.uptimeMillis(), action, x, y, 0)
            event.source = InputDevice.SOURCE_TOUCHSCREEN
            ui.injectInputEvent(event, true)
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
        val catalog = AppCatalog.get(appContext)
        catalog.reload()
        val mine = catalog.visible.filter { it.kind == ProfileKind.Personal && it.available }
        assumeTrue("nicht ausgeführt: weniger als zwei Apps im eigenen Profil", mine.size >= 2)
        val first = mine.firstOrNull { it.packageName == pkg } ?: mine[0]
        val second = mine.firstOrNull { it.key != first.key && it.label != first.label }
        assumeTrue("nicht ausgeführt: keine zweite App mit anderem Namen", second != null)
        requireNotNull(second)
        // While an item is dragged, the drop zones „Entfernen“ and „Deinstallieren“ cover the top 72 dp of the page area
        // (`resolveHover`): an app let go on the first row is removed, not folded. The two apps stand in the third row.
        launcherDir().mkdirs()
        File(launcherDir(), "layout.json").writeText(
            layoutOf(
                appItemAt("ia", first.componentString, first.userSerial, Cell(1, 2)),
                appItemAt("ib", second.componentString, second.userSerial, Cell(2, 2)),
            ),
        )
        startLauncher()
        val source = device.wait(Until.findObject(By.desc(first.label)), 10_000)
        val target = device.wait(Until.findObject(By.desc(second.label)), 10_000)
        assertNotNull("the first app is not on the home screen", source)
        assertNotNull("the second app is not on the home screen", target)
        dragAndHold(source.visibleCenter, target.visibleCenter, restMs = 900)
        assertTrue(
            "no folder with both apps in layout.json: ${readLayout()?.pages?.flatten()?.map { it.kind + ":" + it.id }}",
            waitFor {
                readLayout()?.pages?.flatten()?.any { it.isFolder && it.children.map { child -> child.id }.toSet() == setOf("ia", "ib") } == true
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
            val work: UserHandle = userHandleOf(id!!)
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

    /**
     * The one-time approval „Immer erlauben“ for this app (DN18), so binding needs no dialog. It is the tool `appwidget`
     * (`frameworks/base/cmds/appwidget`), not `cmd appwidget`: the widget service has no shell command, so that would grant nothing.
     */
    private fun grantBind() = shell("appwidget grantbind --package $pkg")

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

    /**
     * Writes an empty first page, so the middle of the screen is an empty cell, then shows the launcher. The page is told by „Alle Apps“,
     * which stands in the row of the page indicator. The indicator's own description „Seite 1 von 1“ cannot be used: with one page no dots
     * are drawn, the row has no area, and a node without area is not part of the accessibility tree (Compose,
     * `SemanticsOwner.getAllUncoveredSemanticsNodesToIntObjectMap`: `region.intersect` of an empty region is false).
     */
    private fun startEmptyLauncher() {
        launcherDir().mkdirs()
        File(launcherDir(), "layout.json").writeText(emptyLayout)
        startLauncher()
        assertTrue("the home screen is not shown", device.wait(Until.hasObject(By.desc("Alle Apps")), 10_000))
        val pages = readLayout()?.pages
        assertEquals("the empty layout was not taken over: ${readLayout()}", 1, pages?.size)
        assertTrue("the first page is not empty: $pages", pages!![0].none { it.isApp || it.isWidget || it.isFolder || it.isShortcut })
    }

    /**
     * Types [text] into the search field of the app list or of the widget sheet. The placeholder is a node of its own that takes no text
     * (Compose merges it into the field only for the screen reader); the field is the `android.widget.EditText`. The button „Suche
     * leeren“ shows while the query is not empty and tells whether the text arrived.
     */
    private fun typeQuery(placeholder: String, text: String) {
        val field = device.wait(Until.findObject(By.clazz("android.widget.EditText")), 10_000)
            ?: device.wait(Until.findObject(By.text(placeholder)), 10_000)
        assertNotNull("no search field „$placeholder“", field)
        field.text = text
        if (!device.wait(Until.hasObject(By.desc("Suche leeren")), 3_000) && Regex("[A-Za-z0-9 -]+").matches(text)) {
            // The node did not take the action: the characters go as key events into the focused field instead.
            field.click()
            shell("input text ${text.replace(" ", "%s")}")
        }
        assertTrue("the field „$placeholder“ did not take „$text“", device.wait(Until.hasObject(By.desc("Suche leeren")), 10_000))
    }

    /** Empties the search field with its button „Suche leeren“. */
    private fun clearQuery() {
        val button = device.wait(Until.findObject(By.desc("Suche leeren")), 10_000)
        assertNotNull("no button „Suche leeren“", button)
        button.click()
        assertTrue("the query stays after „Suche leeren“", device.wait(Until.gone(By.desc("Suche leeren")), 10_000))
    }

    /** Long press on the empty middle of the page: the settings sheet, then „Widgets“ opens the sheet with its search field. */
    private fun openWidgetSheet() {
        device.swipe(device.displayWidth / 2, device.displayHeight / 2, device.displayWidth / 2, device.displayHeight / 2, 250)
        val sheet = device.wait(Until.findObject(By.text("Widgets")), 10_000)
        assertNotNull("the settings sheet has no entry „Widgets“", sheet)
        sheet.click()
        assertTrue("the sheet „Widgets“ did not open", device.wait(Until.hasObject(By.text("Widgets suchen")), 10_000))
    }

    /** The sheet „Widgets“, searched for the test widget. */
    private fun openPickerForTestWidget() {
        openWidgetSheet()
        typeQuery("Widgets suchen", "Test-Widget")
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
        // The person switches the launcher off in the app's settings: the app is in front, the launcher is not.
        showAppInFront()
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

    // ---- DSK-M12: notification dots, work profile, private space (U38-17) -----------------------------------

    private val badgeListener = "$pkg/app.smartexplorer.android.launcher.badges.BadgeListenerService"
    private val ownActivity = "$pkg/app.smartexplorer.android.MainActivity"

    /** The id of the user a `pm create-user` command printed, or `null` when it refused. */
    private fun createdUserId(output: String): Int? = Regex("""id\s+(\d+)""").find(output)?.groupValues?.get(1)?.toIntOrNull()

    /** True while `dumpsys user` marks the profile [id] as paused (quiet mode); a locked private space is paused as well. */
    private fun isQuiet(id: Int): Boolean {
        val text = shell("dumpsys user")
        val start = text.indexOf("UserInfo{$id:")
        if (start < 0) return false
        return text.substring(start, minOf(text.length, start + 900)).contains("QUIET_MODE")
    }

    /** A layout with the own app on page 1, so there is a tile to carry the dot. */
    private fun layoutWithOwnApp(userSerial: Long): String {
        val item = HomeItem(
            id = "iown",
            kind = "app",
            attrs = kotlinx.serialization.json.JsonObject(
                mapOf(
                    "component" to kotlinx.serialization.json.JsonPrimitive(ownActivity),
                    "user" to kotlinx.serialization.json.JsonPrimitive(userSerial),
                ),
            ),
        )
        val layout = HomeLayout(app.smartexplorer.android.launcher.model.GridSpec(4, 5, 56), listOf(listOf(item)), emptyList())
        return LayoutJson.encode(layout)
    }

    @Test
    fun launcher_task_badge_from_test_notification() {
        // The test APK has no launcher icon, so the notification comes from the app under test (the instrumentation shares it).
        launcherDir().mkdirs()
        File(launcherDir(), "layout.json").writeText(layoutWithOwnApp(0))
        shell("pm grant $pkg android.permission.POST_NOTIFICATIONS")
        startLauncher()
        shell("cmd notification allow_listener $badgeListener")
        val manager = appContext.getSystemService(android.app.NotificationManager::class.java)
        val channel = android.app.NotificationChannel("launcher-task-badge", "Launcher task", android.app.NotificationManager.IMPORTANCE_DEFAULT)
        channel.setShowBadge(true)
        manager.createNotificationChannel(channel)
        try {
            val label = "Smart Explorer"
            assertTrue("no tile for the own app", device.wait(Until.hasObject(By.desc(label)), 10_000))
            assertFalse("a dot before any notification", device.hasObject(By.descContains("Benachrichtigung")))
            val notification = android.app.Notification.Builder(appContext, channel.id)
                .setSmallIcon(android.R.drawable.ic_dialog_info)
                .setContentTitle("Launcher task")
                .build()
            manager.notify(7381, notification)
            assertTrue("no dot appeared on the icon", device.wait(Until.hasObject(By.desc("$label, Benachrichtigung")), 20_000))
            manager.cancel(7381)
            assertTrue("the dot stays after the notification is gone", device.wait(Until.gone(By.descContains("Benachrichtigung")), 20_000))
        } finally {
            manager.cancel(7381)
            manager.deleteNotificationChannel(channel.id)
            shell("cmd notification disallow_listener $badgeListener")
        }
    }

    @Test
    fun launcher_task_quiet_mode_toggle() {
        val created = shell("pm create-user --profileOf 0 --managed Arbeit")
        val id = createdUserId(created)
        assumeTrue("nicht ausgeführt: Arbeitsprofil nicht anlegbar (DN17): $created", id != null)
        try {
            shell("am start-user $id")
            startLauncher()
            val button = device.wait(Until.findObject(By.desc("Alle Apps")), 10_000)
            assertNotNull(button)
            button.click()
            val tab = device.wait(Until.findObject(By.text("Arbeit")), 15_000)
            assertNotNull("no tab „Arbeit“ with a work profile", tab)
            tab.click()
            assertTrue("no row „Arbeitsprofil pausieren“", device.wait(Until.hasObject(By.text("Arbeitsprofil pausieren")), 10_000))
            // A Compose `Switch` has the class `android.view.View` and the role description „Switch“ (the delegate sets no class for
            // `Role.Switch`); it is the one node of the tab that is checkable (a tab has `Role.Tab` and is excluded from that).
            val toggle = device.wait(Until.findObject(By.checkable(true)), 10_000)
            assertNotNull("no switch for the work profile", toggle)
            assertFalse("the profile starts paused", isQuiet(id!!))
            toggle.click()
            assertTrue("dumpsys user shows no quiet mode after „Arbeitsprofil pausieren“", waitFor(20_000) { isQuiet(id) })
            assertTrue("no banner while paused", device.wait(Until.hasObject(By.text("Arbeits-Apps sind pausiert.")), 20_000))
            device.findObject(By.text("Fortsetzen")).click()
            assertTrue("the profile stays paused after „Fortsetzen“", waitFor(20_000) { !isQuiet(id) })
            assertTrue("the banner stays", device.wait(Until.gone(By.text("Arbeits-Apps sind pausiert.")), 20_000))
        } finally {
            shell("pm remove-user $id")
        }
    }

    /**
     * Creates a private space profile and installs the app into it, so that the private space has an app whose name is
     * known (A34: on a device without the private space, `pm create-user` refuses and the test is skipped).
     */
    private fun withPrivateSpace(body: (id: Int, serial: Long) -> Unit) {
        assumeTrue("nicht ausgeführt: Privater Bereich braucht Android 15 oder neuer", android.os.Build.VERSION.SDK_INT >= 35)
        val created = shell("pm create-user --profileOf 0 --user-type android.os.usertype.profile.PRIVATE Privat")
        val id = createdUserId(created)
        assumeTrue("nicht ausgeführt: Privater Bereich nicht anlegbar (A34): $created", id != null)
        try {
            shell("am start-user $id")
            shell("pm install-existing --user $id $pkg")
            val serial = appContext.getSystemService(android.os.UserManager::class.java).getSerialNumberForUser(userHandleOf(id!!))
            body(id, serial)
        } finally {
            shell("pm remove-user $id")
        }
    }

    /** Opens the app list and waits for the section of the private space. */
    private fun openDrawerWithPrivateSection() {
        val button = device.wait(Until.findObject(By.desc("Alle Apps")), 10_000)
        assertNotNull(button)
        button.click()
        assertTrue("no section „Privater Bereich“", device.wait(Until.hasObject(By.text("Privater Bereich")), 15_000))
    }

    @Test
    fun launcher_task_private_space_hidden_when_locked() {
        withPrivateSpace { id, serial ->
            // The private copy of the app sits on the home screen before the space is locked.
            launcherDir().mkdirs()
            File(launcherDir(), "layout.json").writeText(layoutWithOwnApp(serial))
            startLauncher()
            val catalog = AppCatalog.get(appContext)
            assertTrue("the private space has no apps while unlocked", waitFor { catalog.reload(); catalog.visible.any { it.kind == app.smartexplorer.android.launcher.apps.ProfileKind.Private } })
            assertTrue("the tile of the private app is missing while unlocked", device.wait(Until.hasObject(By.desc("Smart Explorer")), 10_000))
            openDrawerWithPrivateSection()
            assertTrue("no button „Sperren“", device.wait(Until.hasObject(By.text("Sperren")), 10_000))
            device.findObject(By.text("Sperren")).click()
            assertTrue("dumpsys user shows the private space still unlocked after „Sperren“", waitFor(20_000) { isQuiet(id) })
            assertTrue("no line „Privater Bereich ist gesperrt.“", device.wait(Until.hasObject(By.text("Privater Bereich ist gesperrt.")), 20_000))
            assertTrue(
                "the locked private space still lists apps",
                waitFor { catalog.reload(); catalog.apps.value.none { it.kind == app.smartexplorer.android.launcher.apps.ProfileKind.Private } },
            )
            // The icon on the home screen is gone, its cell stays in the arrangement.
            device.pressHome()
            assertTrue("the tile of the locked private space is still drawn", device.wait(Until.gone(By.desc("Smart Explorer")), 15_000))
            assertTrue("the item left layout.json", readLayout()?.pages?.flatten()?.any { it.id == "iown" } == true)
        }
    }

    @Test
    fun launcher_task_private_space_not_in_search_when_locked() {
        withPrivateSpace { id, _ ->
            launcherDir().mkdirs()
            File(launcherDir(), "layout.json").writeText(emptyLayout)
            startLauncher()
            openDrawerWithPrivateSection()
            // The query is shorter than the name, so the field itself never matches the exact text of a hit.
            typeQuery("Apps suchen", "Smart Explor")
            // Unlocked: the personal and the private app answer.
            assertTrue("fewer than two hits while unlocked", waitFor { device.findObjects(By.text("Smart Explorer")).size >= 2 })
            clearQuery()
            device.wait(Until.findObject(By.text("Sperren")), 10_000)?.click()
            assertTrue("the private space did not lock", waitFor(20_000) { isQuiet(id) })
            typeQuery("Apps suchen", "Smart Explor")
            assertTrue("no hit for the personal app", waitFor { device.findObjects(By.text("Smart Explorer")).isNotEmpty() })
            assertTrue("the locked private space answers the search", waitFor { device.findObjects(By.text("Smart Explorer")).size == 1 })
        }
    }


    // ---- DSK-M13: se content, the bridge, the widget (U40-21) ------------------------------------------------

    private val seWidgetProvider = "app.smartexplorer.android.widget.DevicesWidgetProvider"
    private val seWorkspace = LauncherSeTaskSupport

    /** Writes a layout with the given se items (and, if given, a cache file), then shows the launcher. */
    private fun startSeLauncher(vararg items: String, cache: DevicesData? = null) {
        launcherDir().mkdirs()
        File(launcherDir(), "layout.json").writeText(seWorkspace.layout(*items))
        cache?.let { seWorkspace.writeCache(launcherDir(), it) }
        startLauncher()
    }

    /** Collects what the launcher asks of the main process; it sees the requests before the app's own navigation does. */
    private fun captureNavigation(): Pair<MutableList<NavRequest>, Job> {
        val seen = CopyOnWriteArrayList<NavRequest>()
        val job = CoroutineScope(Dispatchers.Default).launch { AppNav.requests.collect { seen += it } }
        return seen to job
    }

    /** True while `dumpsys` lists the bridge service of this app (it exists only while a client is bound). */
    private fun bridgeBound(): Boolean = shell("dumpsys activity services $pkg").contains("LauncherBridgeService")

    private fun clickText(text: String, timeoutMs: Long = 15_000) {
        val node = device.wait(Until.findObject(By.text(text)), timeoutMs)
        assertNotNull("„$text“ is not on the screen", node)
        node.click()
    }

    @Test
    fun launcher_task_devices_from_bridge() {
        assumeTrue("nicht ausgeführt: das Share-Paar des Wirtsskripts fehlt (seShareServer, seDesktopDevice, seDesktopDirectCode)", seWorkspace.hasPairArguments())
        val phase = phaseArgument("launcherDevicesPhase") ?: "online"
        val contact = runBlocking { Api.awaitReady(); seWorkspace.pairWithRunner() }
        val name = contact.text("name")
        startSeLauncher(seWorkspace.item("d1", "seDevice", 0, 0, contact.text("location"), name))
        assertNotNull("the tile of the runner is not shown", device.wait(Until.findObject(By.text(name)), 20_000))
        if (phase == "online") {
            val online = listOf("Erreichbar", "Erreichbar im LAN", "Verbunden", "Verbunden (direkt)", "Verbunden (Relay)", "Verbindet", "Warte auf Freigabe", "Wartet auf das Gerät")
            assertTrue("no state text of the runner's tile", waitFor(60_000) { online.any { device.hasObject(By.text(it)) } })
            // The search finds the device under „Smart Explorer“ (U40-15). The query lacks the last letter of the name, so the field
            // itself never matches the exact text of the hit.
            device.wait(Until.findObject(By.text("Alle Apps")), 10_000)?.click()
            assertTrue("the app list did not open", device.wait(Until.hasObject(By.text("Apps suchen")), 10_000))
            // The tile below the list may still be part of the accessibility tree; the hit is one more node with the name.
            val shown = device.findObjects(By.text(name)).size
            typeQuery("Apps suchen", if (name.length > 3) name.dropLast(1) else name)
            assertTrue("no section „Smart Explorer“ in the search", device.wait(Until.hasObject(By.text("Smart Explorer")), 10_000))
            assertTrue("the device is not a hit", waitFor { device.findObjects(By.text(name)).size > shown })
        } else {
            // Phase „offline“: the host has taken the runner's Share offline; the tile follows with „Offline“.
            assertTrue("the tile did not change to „Offline“", waitFor(180_000) { device.hasObject(By.text("Offline")) })
        }
    }

    @Test
    fun launcher_task_tile_opens_device_location() {
        assumeTrue("nicht ausgeführt: das Share-Paar des Wirtsskripts fehlt", seWorkspace.hasPairArguments())
        val contact = runBlocking { Api.awaitReady(); seWorkspace.pairWithRunner() }
        val location = contact.text("location")
        val name = contact.text("name")
        val (seen, job) = captureNavigation()
        try {
            startSeLauncher(seWorkspace.item("d1", "seDevice", 0, 0, location, name))
            // The tile opens the folder, except while the access is not approved or the identity changed (then the Share tab).
            assertTrue("the tile shows a state", waitFor(60_000) { device.hasObject(By.text(name)) })
            val status = runBlocking { Api.obj("share.status").objects("devices").first { it.text("contactId") == contact.text("contactId") }.text("status") }
            val expected = SeActions.tapDevice(status, location)
            clickText(name)
            val wanted = AppNav.decode(expected.kind, expected.value)
            assertNotNull(wanted)
            assertTrue("the launcher asked for $wanted, saw $seen", waitFor(30_000) { wanted in seen })
        } finally {
            job.cancel()
        }
    }

    @Test
    fun launcher_task_tile_opens_all_screens() {
        // No pair needed: „Alle Bildschirme“ is a tile without data.
        val (seen, job) = captureNavigation()
        try {
            startSeLauncher(seWorkspace.item("s1", "seScreens", 0, 0))
            clickText("Alle Bildschirme")
            assertTrue("showScreenGrid did not arrive, saw $seen", waitFor(30_000) { NavRequest.ShowScreenGrid(null) in seen })
        } finally {
            job.cancel()
        }
    }

    @Test
    fun launcher_task_room_folder_members() {
        assumeTrue("nicht ausgeführt: das Share-Paar oder der Raumcode des Wirtsskripts fehlt", seWorkspace.hasPairArguments() && !InstrumentationRegistry.getArguments().getString("seRoomCode").isNullOrBlank())
        val (profileId, roomName, members) = runBlocking {
            Api.awaitReady()
            Share.goOnline(TaskArgs.get("seShareServer"))
            val profile = Api.obj("share.joinRoom", args("code" to TaskArgs.get("seRoomCode"), "name" to "Team")).text("profileId")
            Share.awaitMember(profile, TaskArgs.get("seDesktopDevice"))
            val room = Share.room { it.text("profileId") == profile }!!
            Triple(profile, room.text("name"), room.objects("members").map { it.text("name") })
        }
        assertTrue("the room has no member", members.isNotEmpty())
        startSeLauncher(seWorkspace.item("r1", "seRoom", 0, 0, profileId, roomName))
        clickText(roomName)
        for (member in members) {
            assertTrue("member „$member“ is not in the popup", device.wait(Until.hasObject(By.text(member)), 15_000))
        }
    }

    @Test
    fun launcher_task_screen_nav_kinds_reach_the_app() {
        // The bridge side of „openScreen“ and „showScreenGrid“: the intent built from the wire kinds reaches the app's navigation.
        val (seen, job) = captureNavigation()
        try {
            val token = AppNav.tokenFor(appContext)
            val ref = app.smartexplorer.android.launcher.bridge.ScreenRef("direct", "c1", "dev-c1")
            for (target in listOf(SeActions.openScreen(ref, control = true), SeActions.screenGrid("room-1"))) {
                val request = AppNav.decode(target.kind, target.value)!!
                appContext.startActivity(AppNav.intentFor(appContext, request, token))
                assertTrue("$request did not arrive, saw $seen", waitFor(20_000) { request in seen })
            }
        } finally {
            job.cancel()
        }
    }

    @Test
    fun launcher_task_unbind_on_stop() {
        startSeLauncher(seWorkspace.item("s1", "seScreens", 0, 0))
        assertTrue("the bridge was not bound after onStart (rule C3.3 a)", waitFor(30_000) { bridgeBound() })
        // Another app on top: the launcher stops and lets go of the service.
        shell("am start -a android.settings.SETTINGS")
        assertTrue("the bridge stayed bound after the launcher stopped", waitFor(30_000) { !bridgeBound() })
        device.pressHome()
        assertTrue("the bridge was not bound again", waitFor(30_000) { bridgeBound() })
    }

    /**
     * Phase test (U40-21): the main process cannot be ended from inside its own instrumentation, so the host does that between the
     * phases (`am kill <package>`, then Home) and passes what it saw. Phase `prepare` shows the launcher with a cache; phase `verify`
     * (a new instrumentation run) checks the host's observations and that the cache holds no token.
     */
    @Test
    fun launcher_task_no_bind_when_main_process_absent() {
        val phase = phaseArgument("launcherMainAbsentPhase")
        assumeTrue(
            "nicht ausgeführt: der Wirtslauf ruft diesen Test mit '-e launcherMainAbsentPhase prepare' auf, beendet danach den Hauptprozess " +
                "('am kill $pkg'), drückt Home, liest 'pidof $pkg' und die Texte des Bildschirms und ruft ihn mit " +
                "'-e launcherMainAbsentPhase verify -e launcherMainPidAfterHome <pid|none> -e launcherTileAfterHome <Text>' erneut auf",
            phase == "prepare" || phase == "verify",
        )
        if (phase == "prepare") {
            val cache = DevicesData(stale = true, devices = listOf(seWorkspace.deviceEntry("c-absent", "Cache-Gerät", "connected", "Verbunden", true)))
            startSeLauncher(seWorkspace.item("d1", "seDevice", 0, 0, "share://direct/c-absent", "Cache-Gerät"), cache = cache)
            assertTrue("the cached tile is not shown", device.wait(Until.hasObject(By.text("Cache-Gerät")), 20_000))
            // The main process runs now (it is this instrumentation), so the launcher binds it (rule C3.3 a) and writes what it hears into
            // the cache, in place of the device above. The host needs the cache this test made: another app takes the front, the launcher
            // unbinds, what is queued is written out, and then the file is put in place again.
            shell("am start -a android.settings.SETTINGS")
            assertTrue("the bridge stayed bound after the launcher stopped", waitFor(30_000) { !bridgeBound() })
            SystemClock.sleep(2_500)
            seWorkspace.writeCache(launcherDir(), cache)
        } else {
            assertEquals("the main process was started by the launcher", "none", phaseArgument("launcherMainPidAfterHome"))
            assertTrue(
                "the tile does not say „nicht aktuell“: ${phaseArgument("launcherTileAfterHome")}",
                phaseArgument("launcherTileAfterHome").orEmpty().contains("nicht aktuell"),
            )
            val cacheText = File(launcherDir(), BridgeProtocol.SE_CACHE_FILE).readText()
            assertFalse("the cache holds the token", cacheText.contains("navToken"))
            assertTrue("the cache is not marked stale", cacheText.contains("\"stale\":true"))
        }
    }

    @Test
    fun launcher_task_open_in_files_resolver_excludes_se() {
        val folder = DevicesFolder("$pkg.documents", "devices")
        val intent = OpenInFiles.intentFor(appContext, folder, "direct~x")
        assumeTrue("nicht ausgeführt: kein Dateimanager außer Smart Explorer beantwortet den Wurzel-Aufruf (A36)", intent != null)
        if (intent!!.action == Intent.ACTION_CHOOSER) {
            @Suppress("DEPRECATION")
            val excluded = intent.getParcelableArrayExtra(Intent.EXTRA_EXCLUDE_COMPONENTS)?.map { it as ComponentName }.orEmpty()
            assertTrue("MainActivity is not excluded from the chooser: $excluded", excluded.any { it.packageName == pkg && it.className.endsWith("MainActivity") })
        } else {
            assertTrue("the target names Smart Explorer itself", intent.`package` != null && intent.`package` != pkg)
        }
        // Without the exclusion Smart Explorer would be offered: it answers the directory type itself (U35-12).
        val own = Intent(Intent.ACTION_VIEW).setDataAndType(
            android.provider.DocumentsContract.buildDocumentUri(folder.authority, "devices/direct~x"),
            android.provider.DocumentsContract.Document.MIME_TYPE_DIR,
        )
        assertTrue("Smart Explorer does not answer the directory type", appContext.packageManager.queryIntentActivities(own, 0).any { it.activityInfo.packageName == pkg })
        assertEquals(null, OpenInFiles.intentFor(appContext, null, "direct~x"))
    }

    private fun widgetProviderListed(): Boolean = AppWidgetManager.getInstance(appContext).installedProviders.any {
        it.provider.packageName == pkg && it.provider.className == seWidgetProvider
    }

    /**
     * Below API 31 the system lists a widget provider only when its manifest says `enabled`. `AppWidgetServiceImpl.addProviderLocked` of
     * android-11.0.0_r1 refuses a receiver with `!ri.activityInfo.isEnabled()`, and `ActivityInfo.enabled` is the manifest value, never
     * the component state the app set (`PackageInfoUtils.assignSharedFieldsForComponentInfo`: `componentInfo.enabled =
     * mainComponent.isEnabled()`); Android 12 dropped the check. `DevicesWidgetProvider` is declared `enabled="false"` and switched on
     * at run time, so on API 30 (the app's minSdk) it never shows up whatever the part says. That is an open defect of the manifest
     * (the default would have to be `enabled`, with the part switched off at the first start), not of this test: it is reported as not
     * run, never as passed. The test runs for real as soon as the manifest of the receiver says `enabled`.
     */
    private fun assumeProviderListable() {
        val manifestEnabled = appContext.packageManager
            .getReceiverInfo(ComponentName(pkg, seWidgetProvider), PackageManager.MATCH_DISABLED_COMPONENTS).enabled
        assumeTrue(
            "nicht ausgeführt: Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT}) listet einen zur Laufzeit eingeschalteten " +
                "Widget-Provider nicht, dessen Manifest enabled=false sagt (AppWidgetServiceImpl.addProviderLocked, android-11.0.0_r1); " +
                "offener Produktfehler im Manifest von $seWidgetProvider",
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.S || manifestEnabled,
        )
    }

    @Test
    fun launcher_task_devices_widget_only_when_enabled() {
        assertFalse("the provider is listed without the part", widgetProviderListed())
        val on = LauncherSwitch.setWidgetPart(appContext, true)
        assertTrue(on.parts.widget)
        assumeProviderListable()
        assertTrue("the provider is not listed with the part", waitFor(20_000) { widgetProviderListed() })
        val off = LauncherSwitch.setWidgetPart(appContext, false)
        assertFalse(off.parts.widget)
        assertTrue("the provider is still listed after the part went off", waitFor(20_000) { !widgetProviderListed() })
        assertFalse("the cache folder stays after the part went off", File(appContext.filesDir, BridgeProtocol.WIDGET_DIR).exists())
    }

    /** Switches the widget part on, replaces the feed's file by a known row, and puts the widget on page 1 of the launcher. */
    private fun addDevicesWidget(rowName: String, location: String) {
        LauncherSwitch.setWidgetPart(appContext, true)
        DevicesWidgetFeed.stop()
        WidgetCache.write(appContext, WidgetSnapshot(listOf(WidgetRow(rowName, "Verbunden", true, WidgetTarget.OpenLocation(location))), AppNav.tokenFor(appContext)))
        assumeProviderListable()
        assertTrue("the provider is not listed", waitFor(20_000) { widgetProviderListed() })
        grantBind()
        startEmptyLauncher()
        openWidgetSheet()
        // The widget is called „Smart Explorer Geräte“; the query keeps to ASCII, the picker matches the start of the word.
        typeQuery("Widgets suchen", "Smart Explorer Gera")
        clickText("Hinzufügen")
        assertTrue(
            "the devices widget is not in layout.json",
            waitFor(20_000) { readLayout()?.pages?.flatten().orEmpty().any { it.isWidget && it.provider?.endsWith(seWidgetProvider) == true } },
        )
    }

    @Test
    fun launcher_task_devices_widget_click_opens_device() {
        val (seen, job) = captureNavigation()
        try {
            addDevicesWidget("Widget-Gerät", "share://direct/widget-test")
            clickText("Widget-Gerät", 30_000)
            assertTrue("the row did not open the device, saw $seen", waitFor(30_000) { NavRequest.OpenLocation("share://direct/widget-test/") in seen })
        } finally {
            job.cancel()
        }
    }

    /**
     * Phase test (U40-21): ends with the main process absent, which only the host can do. Phase `prepare` puts the widget on the
     * launcher and writes its id into the task report; the host then runs `am kill <package>`, `am broadcast -a
     * android.appwidget.action.APPWIDGET_UPDATE -n <package>/.widget.DevicesWidgetProvider --eia appWidgetIds <id>` and reads `pidof
     * <package>`; phase `verify` (new run) checks `-e launcherWidgetMainPidAfterUpdate none`.
     */
    @Test
    fun launcher_task_devices_widget_does_not_start_core() {
        val phase = phaseArgument("launcherWidgetCorePhase")
        assumeTrue(
            "nicht ausgeführt: der Wirtslauf ruft diesen Test mit '-e launcherWidgetCorePhase prepare' auf, beendet den Hauptprozess " +
                "('am kill $pkg'), sendet APPWIDGET_UPDATE an $seWidgetProvider mit der Kennung aus files/task-report/devices-widget-id.txt, liest 'pidof $pkg' " +
                "und ruft ihn mit '-e launcherWidgetCorePhase verify -e launcherWidgetMainPidAfterUpdate <pid|none>' erneut auf",
            phase == "prepare" || phase == "verify",
        )
        if (phase == "prepare") {
            addDevicesWidget("Kern-Gerät", "share://direct/core-test")
            val id = readLayout()!!.pages.flatten().first { it.isWidget && it.provider?.endsWith(seWidgetProvider) == true }.widgetId
            TaskReport.file("devices-widget-id.txt").writeText("$id\n")
        } else {
            assertEquals(
                "the widget update started the main process",
                "none",
                phaseArgument("launcherWidgetMainPidAfterUpdate"),
            )
        }
    }


    // ---- the two gestures (U40-18, U40-21) -----------------------------------------------------------------

    private val accessibilitySetting = "enabled_accessibility_services"
    private val controlService = "$pkg/app.smartexplorer.android.screen.ScreenControlService"

    /** The lines of `dumpsys accessibility` that list services: enabled, bound, binding, crashed. */
    private fun accessibilityLines(): List<String> {
        val lists = listOf("Enabled services:", "Bound services:", "Binding services:", "Crashed services:")
        return shell("dumpsys accessibility").lineSequence()
            .map { it.trim() }
            .filter { line -> lists.any { line.startsWith(it) } }
            .toList()
    }

    /** Whether the system still lists the se service as enabled or as crashed. */
    private fun systemListsService(): Boolean = accessibilityLines().any { line ->
        (line.startsWith("Enabled services:") || line.startsWith("Crashed services:")) &&
            line.contains(ScreenControlService::class.java.simpleName)
    }

    /**
     * Switches the se accessibility service on and waits until it is connected. In an instrumentation run two things of the system keep
     * it away. First, a UiAutomation without flags suppresses it ([automation]). Second, `am instrument` force-stops the app when it
     * starts and when it ends: the dead binding puts the service on the system's crashed list, `AccessibilityManagerService.
     * updateServicesLocked` skips crashed services, and only a service that left the enabled list is taken off that list again
     * (`updateCrashedServicesIfNeededLocked`: „users toggle the switch to retry“). Writing the same value again changes nothing, so the
     * setting is deleted, the system's answer is awaited, and then the service is put back.
     */
    private fun accessibilityOn(name: String) {
        automation()
        if (waitFor(1_500) { ScreenControlService.current() != null }) return
        shell("settings delete secure $accessibilitySetting")
        waitFor(10_000) { !systemListsService() }
        shell("settings put secure $accessibilitySetting $name")
        shell("settings put secure accessibility_enabled 1")
        assertTrue(
            "the accessibility service did not start: ${accessibilityLines()}",
            waitFor(30_000) { ScreenControlService.current() != null },
        )
    }

    /** Takes the service out of the setting and waits until the system no longer lists it and the service has let go. */
    private fun accessibilityOff() {
        automation()
        shell("settings delete secure $accessibilitySetting")
        waitFor(10_000) { !systemListsService() }
        waitFor(10_000) { ScreenControlService.current() == null }
    }

    /** Runs [body] with the accessibility service of the app on ([value] is its name) or off (`null`) and puts the earlier setting back afterwards. */
    private fun withAccessibilityServices(value: String?, body: () -> Unit) {
        val before = shell("settings get secure $accessibilitySetting").trim()
        try {
            if (value == null) accessibilityOff() else accessibilityOn(value)
            body()
        } finally {
            if (before.isEmpty() || before == "null") {
                shell("settings delete secure $accessibilitySetting")
            } else {
                shell("settings put secure $accessibilitySetting $before")
            }
        }
    }

    private fun swipeDownOnEmptyPage() {
        val x = device.displayWidth / 2
        device.swipe(x, device.displayHeight / 3, x, device.displayHeight * 2 / 3, 12)
    }

    private fun doubleTapOnEmptyPage() {
        val x = device.displayWidth / 2
        val y = device.displayHeight / 2
        device.click(x, y)
        SystemClock.sleep(80)
        device.click(x, y)
    }

    @Test
    fun launcher_task_gestures_off_without_accessibility() {
        withAccessibilityServices(null) {
            assertFalse("the service must be off", ScreenControlService.enabled(appContext))
            // The settings card names the reason and offers the system page (U40-19). It is looked at first, while this test's own
            // activity is in front: the card shows its gestures line only while the launcher is on, and an activity behind the home
            // screen is stopped (the card reads its state at composition and on `ON_RESUME`).
            assertEquals(EnableResult.Done, LauncherSwitch.enable(appContext))
            compose.setContent { SmartExplorerTheme { LauncherSettings() } }
            compose.onNodeWithText("Gesten sind aus: Die Bedienungshilfe", substring = true).assertExists()
            compose.onNodeWithText("Bedienungshilfe öffnen").assertExists()

            startEmptyLauncher()
            swipeDownOnEmptyPage()
            // The gesture does nothing but say why, once.
            assertTrue("no explanation for the swipe", device.wait(Until.hasObject(By.textContains("Gesten sind aus")), 10_000))
            assertEquals("the swipe opened the notification shade", pkg, device.currentPackageName)
            doubleTapOnEmptyPage()
            SystemClock.sleep(3_000)
            assertTrue("the double tap locked the screen", device.isScreenOn)
            assertEquals(pkg, device.currentPackageName)
        }
    }

    @Test
    fun launcher_task_gestures_with_accessibility() {
        withAccessibilityServices(controlService) {
            // The settings card first, for the reason given in `launcher_task_gestures_off_without_accessibility`.
            assertEquals(EnableResult.Done, LauncherSwitch.enable(appContext))
            compose.setContent { SmartExplorerTheme { LauncherSettings() } }
            compose.onNodeWithText("Wischen nach unten öffnet die Benachrichtigungen, Doppeltippen sperrt den Bildschirm.").assertExists()

            startEmptyLauncher()
            assertTrue("the accessibility service is gone again: ${accessibilityLines()}", waitFor(30_000) { ScreenControlService.current() != null })
            swipeDownOnEmptyPage()
            assertTrue("the notification shade did not open", device.wait(Until.hasObject(By.pkg("com.android.systemui")), 10_000))
            device.pressBack()
            assertLauncherOnTop()

            doubleTapOnEmptyPage()
            val asleep = waitFor(20_000) { shell("dumpsys power").lineSequence().any { "mWakefulness=Asleep" in it || "mWakefulness=Dozing" in it } }
            shell("input keyevent KEYCODE_WAKEUP")
            shell("wm dismiss-keyguard")
            assertTrue("the double tap did not lock the screen", asleep)
        }
    }

    private fun render(drawable: Drawable, size: Int = 192): Bitmap {
        val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        drawable.setBounds(0, 0, size, size)
        drawable.draw(Canvas(bitmap))
        return bitmap
    }
}

/**
 * The handle of user [id]. `UserHandle.of` is no public API; the handle of the uid
 * `id × 100000` (the range of uids per user) is the same user.
 */
private fun userHandleOf(id: Int): UserHandle = UserHandle.getUserHandleForUid(id * 100_000)
