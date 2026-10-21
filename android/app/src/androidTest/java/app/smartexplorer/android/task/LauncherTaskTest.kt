package app.smartexplorer.android.task

import android.app.ActivityManager
import android.content.ComponentName
import android.content.pm.PackageManager
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import app.smartexplorer.android.core.Core
import app.smartexplorer.android.core.CoreState
import app.smartexplorer.android.launcher.EnableResult
import app.smartexplorer.android.launcher.HomeResolution
import app.smartexplorer.android.launcher.LauncherState
import app.smartexplorer.android.launcher.LauncherSwitch
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
 * DSK-M9 and DSK-M10 on the device (U35-34): the launcher module as a whole. The tests run in the app
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
}
