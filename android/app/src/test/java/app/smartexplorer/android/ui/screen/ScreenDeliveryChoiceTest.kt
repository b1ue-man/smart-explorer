package app.smartexplorer.android.ui.screen

import app.smartexplorer.android.core.ScreenViewEvent
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * „Übertragung“ and the measured „Datenrate“ of the Android viewer (user 2026-11-25): the choice stored with the codec
 * choice, and the texts the desktop viewer shows for the host's statistics. Pure functions only.
 */
class ScreenDeliveryChoiceTest {
    @Test
    fun delivery_task_android_choice_is_stored_with_the_codec_choice() {
        // The preferences of an older app have no key: „Echtzeit“.
        assertEquals("realtime", ScreenViewPrefs.decode("hevc", null, null, legacyLossless = false).delivery)
        assertEquals(
            ViewerCodecChoice(codec = "hevc", delivery = "reliable"),
            ScreenViewPrefs.decode("hevc", null, null, legacyLossless = false, delivery = "reliable"),
        )
        // A name this version does not know is „Echtzeit“; the keys are those of the core and `--transport`.
        assertEquals("realtime", ViewerCodecChoice.of(null, null, null, "tcp").delivery)
        assertEquals(listOf("realtime", "reliable"), ViewerCodecChoice.DELIVERIES)
        // A new „Übertragung“ goes out live and never opens a new session.
        val open = ViewerCodecChoice(codec = "h264")
        assertFalse(ViewerCodecChoice.needsReopen(open, open.copy(delivery = "reliable")))
    }

    @Test
    fun delivery_task_android_texts_match_desktop() {
        assertEquals("Echtzeit (wie UDP)", ScreenDelivery.label("realtime"))
        assertEquals("Zuverlässig (wie TCP)", ScreenDelivery.label("reliable"))
        assertEquals("Jedes Bild kommt an; bei langsamer Leitung wird es später, nicht verworfen.", ScreenDelivery.hint("reliable"))
        assertEquals("Ein Bild, das zu spät käme, wird verworfen; das Bild bleibt aktuell.", ScreenDelivery.hint("realtime"))
        val body = Json.parseToJsonElement(
            """{"targetKbps":9000,"chain":{"delivery":"reliable","measuredKbps":48200,"measuredAgeMs":3400}}""",
        ).jsonObject
        val report = DeliveryReport.of(body)
        assertEquals(DeliveryReport("reliable", 48_200, 3_400), report)
        assertEquals(report, ScreenSessionState().apply(ScreenViewEvent.Stats(7, body)).delivery)
        assertEquals(
            listOf("Übertragung Zuverlässig", "Datenrate gemessen 48,2 Mbit/s (vor 3 s)"),
            ScreenDelivery.infoParts(report, statsSeen = true),
        )
        assertEquals("Automatisch (gemessen: 48,2 Mbit/s)", ScreenDelivery.automaticText("Automatisch", report))
        assertEquals("gemessen 48,2 Mbit/s (vor 3 s)", ScreenDelivery.measuredLine(report, statsSeen = true))
        // A host before „Übertragung“: no mode, so neither a change nor a measurement.
        val older = DeliveryReport.of(Json.parseToJsonElement("""{"chain":{"rateLimit":"link"}}""").jsonObject)
        assertNull(older.running)
        assertEquals(listOf("Übertragung nicht gemeldet (ältere Version)"), ScreenDelivery.infoParts(older, statsSeen = true))
        assertEquals("Automatisch", ScreenDelivery.automaticText("Automatisch", older))
        assertEquals("das Gerät misst die Leitung nicht (ältere Version)", ScreenDelivery.measuredLine(older, statsSeen = true))
        // Measuring without a result yet; before the first statistics the line says nothing.
        val fresh = DeliveryReport("realtime", null, null)
        assertEquals(listOf("Übertragung Echtzeit", "Datenrate noch nicht gemessen"), ScreenDelivery.infoParts(fresh, statsSeen = true))
        assertEquals("Automatisch (noch nicht gemessen)", ScreenDelivery.automaticText("Automatisch", fresh))
        assertEquals(emptyList<String>(), ScreenDelivery.infoParts(null, statsSeen = false))
        assertEquals("wartet auf die erste Statistik des Geräts", ScreenDelivery.measuredLine(null, statsSeen = false))
        assertEquals("1,5", ScreenDelivery.mbitText(1_500))
    }
}
