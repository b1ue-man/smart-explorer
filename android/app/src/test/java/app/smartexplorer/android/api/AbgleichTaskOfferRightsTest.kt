package app.smartexplorer.android.api

import app.smartexplorer.android.core.Core
import app.smartexplorer.android.ui.share.offerRightsText
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Plan 2026-10-31 §F5: `share.status` names the rights of the running PIN offer, so the phone
 * shows what was really given after the app was restarted, not „wartet ohne Rechte“.
 */
class AbgleichTaskOfferRightsTest {
    private fun decode(offer: String): DiscoveryOffer? = Core.json.decodeFromString(
        ShareStatus.serializer(),
        """{"identity":{},"discovery":{"offer":$offer,"advertisements":[]}}""",
    ).discovery.offer

    @Test
    fun abgleichTaskTheOfferCarriesTheRightsTheWorkerHolds() {
        val offer = decode(
            """{"offerId":"o1","target":"direct","alias":"Pixel","untilMs":1790000300000,
               "rights":{"folders":{"kind":"all"},"write":true,"commands":"allow","terminal":"allow","screenView":"off",
                         "screenControl":"off","validUntil":null,"account":"own","preset":"files-exec",
                         "summary":"Dateien und Codeausführung"}}""",
        )
        assertNotNull(offer)
        assertEquals("o1", offer!!.offerId)
        val rights = offer.rights!!
        assertEquals("files-exec", rights.preset)
        assertEquals(FOLDERS_ALL, rights.folders.kind)
        assertTrue(rights.write)
        assertEquals(ACCESS_ALLOW, rights.commands)
        assertEquals(ACCESS_ALLOW, rights.terminal)
        assertEquals(ACCESS_OFF, rights.screenView)
    }

    @Test
    fun abgleichTaskAnOfferWithoutRightsAndAnOlderBridgeBothMeanTheDeviceWaits() {
        assertNull(decode("""{"offerId":"o1","target":"direct","alias":"Pixel","untilMs":1,"rights":null}""")!!.rights)
        assertNull(decode("""{"offerId":"o1","target":"direct","alias":"Pixel","untilMs":1}""")!!.rights)
    }

    @Test
    fun abgleichTaskTheOfferLineSaysWhatTheDeviceGets() {
        assertTrue(offerRightsText(null).contains("wartet ohne Rechte"))
        val readOnly = Rights(folders = RightsFolders(FOLDERS_ALL), write = false)
        assertEquals(
            "Das Gerät, das mit dieser PIN verbindet, erhält: Dateien nur lesen (alle Ordner).",
            offerRightsText(readOnly),
        )
        assertTrue(offerRightsText(Rights(preset = PRESET_NONE)).contains("ohne Rechte verbunden"))
        // A room's offer carries none: its own rights apply.
        assertTrue(offerRightsText(null, room = true).contains("Raum"))
    }
}
