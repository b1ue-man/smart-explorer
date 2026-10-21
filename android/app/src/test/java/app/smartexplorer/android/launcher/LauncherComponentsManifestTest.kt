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
}
