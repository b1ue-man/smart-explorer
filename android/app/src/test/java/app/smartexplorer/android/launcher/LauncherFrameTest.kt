package app.smartexplorer.android.launcher

import app.smartexplorer.android.ui.AppNav
import app.smartexplorer.android.ui.MainTab
import app.smartexplorer.android.ui.NavRequest
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.w3c.dom.Element

/**
 * DSK-M9 on the JVM, without Robolectric: the launcher package keeps out of the main process's code
 * (U35-11), the navigation kinds of the other packages still round-trip (U35-14), and the four
 * filters of `MainActivity` stay as they were (U35-12). The module directory is the working directory.
 */
class LauncherFrameTest {
    private val androidNs = "http://schemas.android.com/apk/res/android"

    /** Packages whose code needs the Rust core or the main process's state (U35-11). */
    private val forbiddenPackages = listOf("core", "prefs", "service", "api", "system", "work", "update", "screen", "ui")

    @Test
    fun launcher_task_launcher_package_imports_isolated() {
        val root = File("src/main/java/app/smartexplorer/android/launcher")
        assertTrue("launcher package missing: ${root.absolutePath}", root.isDirectory)
        val files = root.walkTopDown().filter { it.isFile && it.extension == "kt" }.toList()
        assertTrue("no launcher source found", files.isNotEmpty())
        val violations = ArrayList<String>()
        for (file in files) {
            file.readLines().forEachIndexed { index, line ->
                val trimmed = line.trim()
                if (trimmed.startsWith("import ")) {
                    val target = trimmed.removePrefix("import ").substringBefore(" as ").trim()
                    val prefix = "app.smartexplorer.android."
                    if (target.startsWith(prefix)) {
                        val rest = target.removePrefix(prefix)
                        val top = rest.substringBefore('.')
                        val allowedNav = target == "app.smartexplorer.android.ui.AppNav"
                        if (top in forbiddenPackages && !allowedNav) violations += "${file.name}:${index + 1}: $trimmed"
                        if (rest == "MainActivity") violations += "${file.name}:${index + 1}: $trimmed"
                    }
                } else if (!trimmed.startsWith("*") && !trimmed.startsWith("//") && !trimmed.startsWith("/*")) {
                    if (Regex("""\bMainActivity\s*::\s*class""").containsMatchIn(trimmed)) {
                        violations += "${file.name}:${index + 1}: $trimmed"
                    }
                }
            }
        }
        assertTrue("imports the launcher process must not load:\n" + violations.joinToString("\n"), violations.isEmpty())
    }

    @Test
    fun launcher_task_existing_nav_kinds_unchanged() {
        val requests = listOf(
            NavRequest.OpenLocation("sftp://host/path"),
            NavRequest.SelectTab(MainTab.Files),
            NavRequest.SelectTab(MainTab.Sync),
            NavRequest.SelectTab(MainTab.Share),
            NavRequest.SelectTab(MainTab.More),
            NavRequest.ShowTransfers,
            NavRequest.OpenJobConflicts("job-1"),
            NavRequest.ShowShareRequests,
            NavRequest.ShowUpdate,
            NavRequest.ShowBackgroundSettings,
            NavRequest.OpenScreen("{\"device\":\"d1\"}"),
            NavRequest.ShowScreenGrid("room-1"),
            NavRequest.ShowScreenGrid(null),
            NavRequest.OpenVaultCompare("job-2"),
            NavRequest.ShowLauncherSettings,
        )
        for (request in requests) {
            val (kind, value) = AppNav.encode(request)
            assertEquals("round trip of $request", request, AppNav.decode(kind, value))
        }
        // The wire names are part of the contract with notifications and other packages.
        assertEquals("showLauncherSettings" to null, AppNav.encode(NavRequest.ShowLauncherSettings))
        assertEquals("openScreen" to "t", AppNav.encode(NavRequest.OpenScreen("t")))
        assertEquals("showScreenGrid" to "", AppNav.encode(NavRequest.ShowScreenGrid(null)))
        assertEquals("openVaultCompare" to "j", AppNav.encode(NavRequest.OpenVaultCompare("j")))
        // The conflict kind keeps carrying a vault job as `vault:<job>` (U31-13).
        assertEquals(NavRequest.OpenVaultCompare("j"), AppNav.decode("openJobConflicts", "vault:j"))
        assertEquals(NavRequest.OpenJobConflicts("plain"), AppNav.decode("openJobConflicts", "plain"))
        // Unknown kinds and missing values stay refused.
        assertEquals(null, AppNav.decode("openSomethingNew", "x"))
        assertEquals(null, AppNav.decode(null, null))
        assertEquals(null, AppNav.decode("openLocation", null))
        assertEquals(null, AppNav.decode("openScreen", " "))
        assertEquals(MainTab.More, AppNav.tabFor(NavRequest.ShowLauncherSettings))
        assertEquals(MainTab.Share, AppNav.tabFor(NavRequest.ShowScreenGrid(null)))
    }

    @Test
    fun launcher_task_main_activity_filters_unchanged() {
        val activity = activity(".MainActivity")
        val filters = filtersOf(activity).sorted()
        val expected = listOf(
            "action=android.intent.action.MAIN;category=android.intent.category.LAUNCHER;data=",
            "action=android.intent.action.SEND;category=android.intent.category.DEFAULT;data=mime:*/*",
            "action=android.intent.action.SEND_MULTIPLE;category=android.intent.category.DEFAULT;data=mime:*/*",
            "action=android.intent.action.VIEW;category=android.intent.category.DEFAULT;" +
                "data=mime:inode/directory,mime:resource/folder,mime:vnd.android.document/directory",
        ).sorted()
        assertEquals(expected, filters)
        assertEquals("true", activity.getAttributeNS(androidNs, "exported"))
        assertEquals("singleTop", activity.getAttributeNS(androidNs, "launchMode"))
    }

    @Test
    fun manifestDeclaresTheLauncherActivityOffInItsOwnProcess() {
        val activity = activity(".launcher.LauncherActivity")
        assertEquals("false", activity.getAttributeNS(androidNs, "enabled"))
        assertEquals(":launcher", activity.getAttributeNS(androidNs, "process"))
        assertEquals("true", activity.getAttributeNS(androidNs, "exported"))
        assertEquals("singleTask", activity.getAttributeNS(androidNs, "launchMode"))
        assertEquals("", activity.getAttributeNS(androidNs, "taskAffinity"))
        assertEquals(
            listOf("action=android.intent.action.MAIN;category=android.intent.category.DEFAULT,android.intent.category.HOME;data="),
            filtersOf(activity),
        )
        // Every class the main process switches must exist, or setComponentEnabledSetting throws (U35-2).
        for (name in LauncherParts.launcher + LauncherParts.widget) {
            assertNotNull("class $name", Class.forName(name, false, javaClass.classLoader))
        }
    }

    private fun activity(wanted: String): Element {
        val factory = DocumentBuilderFactory.newInstance().apply { isNamespaceAware = true }
        val doc = factory.newDocumentBuilder().parse(File("src/main/AndroidManifest.xml"))
        val activities = doc.getElementsByTagName("activity")
        for (i in 0 until activities.length) {
            val element = activities.item(i) as Element
            if (element.getAttributeNS(androidNs, "name") == wanted) return element
        }
        throw AssertionError("activity $wanted missing in AndroidManifest.xml")
    }

    /** One text per intent filter: its actions, categories and data, each sorted. */
    private fun filtersOf(activity: Element): List<String> {
        val filters = activity.getElementsByTagName("intent-filter")
        return (0 until filters.length).map { index ->
            val filter = filters.item(index) as Element
            fun names(tag: String, attr: String = "name"): List<String> {
                val list = filter.getElementsByTagName(tag)
                return (0 until list.length).map { (list.item(it) as Element).getAttributeNS(androidNs, attr) }.sorted()
            }
            val data = names("data", "mimeType").map { "mime:$it" }
            "action=${names("action").joinToString(",")};category=${names("category").joinToString(",")};data=${data.joinToString(",")}"
        }
    }
}
