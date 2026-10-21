package app.smartexplorer.android.launcher

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.w3c.dom.Element

/**
 * The launcher components of U38 in the manifest (U38-2, U38-3), read as XML without Android: each is off, in the
 * launcher process, and listed in [LauncherParts] so the switch turns it on and off with the launcher (E7).
 */
class LauncherComponentsManifestTest {
    private val androidNs = "http://schemas.android.com/apk/res/android"

    private fun component(tag: String, wanted: String): Element {
        val factory = DocumentBuilderFactory.newInstance().apply { isNamespaceAware = true }
        val doc = factory.newDocumentBuilder().parse(File("src/main/AndroidManifest.xml"))
        val list = doc.getElementsByTagName(tag)
        for (i in 0 until list.length) {
            val element = list.item(i) as Element
            if (element.getAttributeNS(androidNs, "name") == wanted) return element
        }
        throw AssertionError("$tag $wanted missing in AndroidManifest.xml")
    }

    private fun filters(component: Element): List<String> {
        val filters = component.getElementsByTagName("intent-filter")
        return (0 until filters.length).map { index ->
            val filter = filters.item(index) as Element
            fun names(tag: String): List<String> {
                val list = filter.getElementsByTagName(tag)
                return (0 until list.length).map { (list.item(it) as Element).getAttributeNS(androidNs, "name") }.sorted()
            }
            "action=${names("action").joinToString(",")};category=${names("category").joinToString(",")}"
        }
    }

    @Test
    fun thePinRequestActivityIsOffInTheLauncherProcessWithTheTwoSystemFilters() {
        val activity = component("activity", ".launcher.pin.PinRequestActivity")
        assertEquals("false", activity.getAttributeNS(androidNs, "enabled"))
        assertEquals("true", activity.getAttributeNS(androidNs, "exported"))
        assertEquals(":launcher", activity.getAttributeNS(androidNs, "process"))
        assertEquals("singleTask", activity.getAttributeNS(androidNs, "launchMode"))
        assertEquals("", activity.getAttributeNS(androidNs, "taskAffinity"))
        assertEquals("true", activity.getAttributeNS(androidNs, "excludeFromRecents"))
        assertEquals("@android:style/Theme.Translucent.NoTitleBar", activity.getAttributeNS(androidNs, "theme"))
        assertEquals("@string/launcher_pin_label", activity.getAttributeNS(androidNs, "label"))
        assertEquals(
            listOf(
                "action=android.content.pm.action.CONFIRM_PIN_SHORTCUT;category=android.intent.category.DEFAULT",
                "action=android.content.pm.action.CONFIRM_PIN_APPWIDGET;category=android.intent.category.DEFAULT",
            ),
            filters(activity),
        )
        assertTrue("the switch must turn it on and off with the launcher", "app.smartexplorer.android.launcher.pin.PinRequestActivity" in LauncherParts.launcher)
    }

    @Test
    fun theBadgeListenerIsOffInTheLauncherProcessAndBoundOnlyBySystem() {
        val service = component("service", ".launcher.badges.BadgeListenerService")
        assertEquals("false", service.getAttributeNS(androidNs, "enabled"))
        assertEquals("true", service.getAttributeNS(androidNs, "exported"))
        assertEquals(":launcher", service.getAttributeNS(androidNs, "process"))
        assertEquals("android.permission.BIND_NOTIFICATION_LISTENER_SERVICE", service.getAttributeNS(androidNs, "permission"))
        assertEquals("@string/launcher_badges_label", service.getAttributeNS(androidNs, "label"))
        val list = service.getElementsByTagName("action")
        assertEquals(1, list.length)
        assertEquals("android.service.notification.NotificationListenerService", (list.item(0) as Element).getAttributeNS(androidNs, "name"))
        assertTrue("the switch must turn it on and off with the launcher", "app.smartexplorer.android.launcher.badges.BadgeListenerService" in LauncherParts.launcher)
    }

    @Test
    fun theBridgeServiceIsOffInTheMainProcessAndNotExported() {
        val service = component("service", ".service.LauncherBridgeService")
        assertEquals("false", service.getAttributeNS(androidNs, "enabled"))
        assertEquals("false", service.getAttributeNS(androidNs, "exported"))
        // No android:process: the service runs in the main process, where the core lives (U40-3).
        assertEquals("", service.getAttributeNS(androidNs, "process"))
        assertEquals(0, service.getElementsByTagName("intent-filter").length)
        assertTrue("the switch must turn it on and off with the launcher", "app.smartexplorer.android.service.LauncherBridgeService" in LauncherParts.launcher)
        assertEquals(
            "app.smartexplorer.android.service.LauncherBridgeService",
            app.smartexplorer.android.launcher.bridge.BridgeProtocol.SERVICE_CLASS,
        )
    }

    @Test
    fun theWidgetProviderIsOffInTheLauncherProcessAndAnswersOnlyTheUpdateAction() {
        val receiver = component("receiver", ".widget.DevicesWidgetProvider")
        assertEquals("false", receiver.getAttributeNS(androidNs, "enabled"))
        assertEquals("true", receiver.getAttributeNS(androidNs, "exported"))
        assertEquals(":launcher", receiver.getAttributeNS(androidNs, "process"))
        assertEquals("@string/devices_widget_label", receiver.getAttributeNS(androidNs, "label"))
        val actions = receiver.getElementsByTagName("action")
        assertEquals(1, actions.length)
        assertEquals("android.appwidget.action.APPWIDGET_UPDATE", (actions.item(0) as Element).getAttributeNS(androidNs, "name"))
        val meta = receiver.getElementsByTagName("meta-data").item(0) as Element
        assertEquals("android.appwidget.provider", meta.getAttributeNS(androidNs, "name"))
        assertEquals("@xml/devices_widget_info", meta.getAttributeNS(androidNs, "resource"))
    }

    @Test
    fun theWidgetListServiceIsOffInTheLauncherProcessAndBoundOnlyBySystem() {
        val service = component("service", ".widget.DevicesWidgetService")
        assertEquals("false", service.getAttributeNS(androidNs, "enabled"))
        assertEquals("false", service.getAttributeNS(androidNs, "exported"))
        assertEquals(":launcher", service.getAttributeNS(androidNs, "process"))
        assertEquals("android.permission.BIND_REMOTEVIEWS", service.getAttributeNS(androidNs, "permission"))
    }

    @Test
    fun theWidgetPartSwitchesExactlyTheProviderAndItsService() {
        assertEquals(
            listOf(
                "app.smartexplorer.android.widget.DevicesWidgetProvider",
                "app.smartexplorer.android.widget.DevicesWidgetService",
            ),
            LauncherParts.widget,
        )
        // The parts do not overlap: switching the widget part never touches the launcher components.
        assertTrue(LauncherParts.launcher.none { it in LauncherParts.widget })
    }

    @Test
    fun theWidgetInfoFilesCarryTheSizesOfTheDevicesWidget() {
        for ((path, v31) in listOf("src/main/res/xml/devices_widget_info.xml" to false, "src/main/res/xml-v31/devices_widget_info.xml" to true)) {
            val factory = DocumentBuilderFactory.newInstance().apply { isNamespaceAware = true }
            val info = factory.newDocumentBuilder().parse(File(path)).documentElement
            fun attr(name: String) = info.getAttributeNS(androidNs, name)
            assertEquals("180dp", attr("minWidth"))
            assertEquals("110dp", attr("minHeight"))
            assertEquals("110dp", attr("minResizeWidth"))
            assertEquals("110dp", attr("minResizeHeight"))
            assertEquals("0", attr("updatePeriodMillis"))
            assertEquals("@layout/devices_widget", attr("initialLayout"))
            assertEquals("horizontal|vertical", attr("resizeMode"))
            assertEquals("home_screen", attr("widgetCategory"))
            assertEquals("@string/devices_widget_description", attr("description"))
            assertEquals(if (v31) "3" else "", attr("targetCellWidth"))
            assertEquals(if (v31) "2" else "", attr("targetCellHeight"))
            assertEquals(if (v31) "@layout/devices_widget" else "", attr("previewLayout"))
        }
    }
}
