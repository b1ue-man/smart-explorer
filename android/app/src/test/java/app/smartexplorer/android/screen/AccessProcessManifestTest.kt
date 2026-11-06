package app.smartexplorer.android.screen

import app.smartexplorer.android.SmartExplorerApp
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Test
import org.w3c.dom.Element

/**
 * The accessibility service and its link run in their own process (manifest read as XML): an
 * enabled accessibility service stays bound for good, and in the main process it kept the core,
 * the daemon and Share alive in the background. The application skips its start-up there.
 */
class AccessProcessManifestTest {
    private val androidNs = "http://schemas.android.com/apk/res/android"

    private fun service(name: String): Element {
        val factory = DocumentBuilderFactory.newInstance().apply { isNamespaceAware = true }
        val list = factory.newDocumentBuilder().parse(File("src/main/AndroidManifest.xml")).getElementsByTagName("service")
        for (i in 0 until list.length) {
            val element = list.item(i) as Element
            if (element.getAttributeNS(androidNs, "name") == name) return element
        }
        throw AssertionError("service $name missing in AndroidManifest.xml")
    }

    @Test
    fun theAccessibilityServiceRunsInTheAccessProcessAndOnlyTheSystemBindsIt() {
        val service = service(".screen.ScreenControlService")
        assertEquals(SmartExplorerApp.ACCESS_PROCESS_SUFFIX, service.getAttributeNS(androidNs, "process"))
        assertEquals("android.permission.BIND_ACCESSIBILITY_SERVICE", service.getAttributeNS(androidNs, "permission"))
    }

    @Test
    fun theLinkRunsBesideItAndIsNotExported() {
        val link = service(".screen.AccessLinkService")
        assertEquals(SmartExplorerApp.ACCESS_PROCESS_SUFFIX, link.getAttributeNS(androidNs, "process"))
        assertEquals("false", link.getAttributeNS(androidNs, "exported"))
        assertEquals(0, link.getElementsByTagName("intent-filter").length)
    }
}
