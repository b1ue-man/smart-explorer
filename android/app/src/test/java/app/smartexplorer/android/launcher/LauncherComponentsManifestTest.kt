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
        assertTrue("the widget part stays empty until U40", LauncherParts.widget.isEmpty())
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
}
