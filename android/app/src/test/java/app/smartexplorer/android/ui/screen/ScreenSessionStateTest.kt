package app.smartexplorer.android.ui.screen

import app.smartexplorer.android.core.CoreEvents
import app.smartexplorer.android.core.ScreenViewEvent
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The events of a viewer session, the state they build, the target JSON and the zoom (SCR-M9). */
class ScreenSessionStateTest {
    private fun event(text: String): ScreenViewEvent =
        CoreEvents.decodeScreenView(Json.parseToJsonElement(text) as JsonObject) ?: throw AssertionError("not an event: $text")

    @Test
    fun the_events_of_a_session_build_its_state() {
        var state = ScreenSessionState()
        assertEquals("connecting", state.state)
        state = state.apply(event("""{"type":"screenView","session":3,"kind":"pending","capability":"screenControl"}"""))
        assertEquals("screenControl", state.pending)
        assertEquals("waiting_for_approval", state.state)
        state = state.apply(
            event(
                """{"type":"screenView","session":3,"kind":"accepted","path":"relay","platform":"android",
                   "authorization":{"view":"allow","control":"ask"},
                   "input":{"touch":true,"actions":["back","home"],"toolbarKeys":[227]},
                   "monitors":[{"id":"0","name":"","x":0,"y":0,"width":1080,"height":2400}],"limits":["no_key_events"]}""",
            ),
        )
        assertTrue(state.accepted)
        assertNull(state.pending)
        assertEquals("starting", state.state)
        assertEquals("relay", state.path)
        assertEquals("ask", state.control)
        assertTrue(state.input.touch)
        assertEquals(listOf("back", "home"), state.input.actions)
        assertEquals(listOf(227), state.input.toolbarKeys)
        assertEquals(1080, state.monitors.single().width)
        assertEquals(listOf("no_key_events"), state.limits)
        assertTrue("touch and a control right allow „Steuern“", state.canControl)
        state = state.apply(event("""{"type":"screenView","session":3,"kind":"stream","codec":"h264","profile":"high","width":1080,"height":2400,"gop":65538,"encoder":"c2","hardware":true}"""))
        assertEquals("the low 16 bits of the group number", 2, state.streams.keys.single())
        assertEquals(1080, state.streams.getValue(2).width)
        state = state.apply(event("""{"type":"screenView","session":3,"kind":"state","state":"streaming","reason":null}"""))
        assertEquals("streaming", state.state)
        state = state.apply(event("""{"type":"screenView","session":3,"kind":"controlState","holder":"other","blocked":"secure_desktop"}"""))
        assertEquals("other", state.holder)
        assertEquals("secure_desktop", state.blocked)
        state = state.apply(event("""{"type":"screenView","session":3,"kind":"windows","windows":[{"id":"7","title":"Notizen","app":"notes"}],"apps":[{"appId":"a.b","name":"B"}],"listSource":"enum_windows","limits":[]}"""))
        assertEquals("Notizen", state.windows?.windows?.single()?.title)
        state = state.apply(event("""{"type":"screenView","session":3,"kind":"error","code":"control_busy","detail":null}"""))
        assertEquals("control_busy", state.errorCode)
        state = state.apply(event("""{"type":"screenView","session":3,"kind":"ended","code":"host_stopped","detail":null}"""))
        assertEquals("host_stopped", state.ended?.code)
        assertEquals("ended", state.state)
    }

    @Test
    fun a_host_that_forbids_control_or_has_no_input_cannot_be_controlled() {
        val accepted = ScreenSessionState(accepted = true, control = "off", input = app.smartexplorer.android.api.ScreenInputCaps(pointer = true))
        assertFalse(accepted.canControl)
        assertFalse(accepted.copy(control = "allow", input = app.smartexplorer.android.api.ScreenInputCaps()).canControl)
        assertTrue(accepted.copy(control = "allow").canControl)
        assertFalse(ScreenSessionState(control = "allow", input = accepted.input).canControl)
    }

    @Test
    fun the_stream_cache_keeps_the_newest_groups() {
        var state = ScreenSessionState()
        for (gop in 1L..40L) {
            state = state.apply(ScreenViewEvent.Stream(1, "h264", "cb", 100, 50, gop, "e", false))
        }
        assertEquals(ScreenSessionState.STREAM_CACHE, state.streams.size)
        assertTrue(40 in state.streams && 25 in state.streams && 24 !in state.streams)
    }

    @Test
    fun the_target_json_has_the_form_of_contract_v5() {
        val target = ScreenTarget("room", "profile-1", "device-9", source = "window:5", control = true)
        assertEquals(
            """{"relation":"room","relation_id":"profile-1","device_id":"device-9","source":"window:5","control":true}""",
            target.toJson(),
        )
        assertEquals(target, ScreenTargetJson.parse(target.toJson()).getOrThrow())
        val direct = ScreenTargetJson.parse("""{"relation":"direct","relation_id":"c1","device_id":null,"source":null,"control":false}""").getOrThrow()
        assertEquals(ScreenTarget("direct", "c1", null), direct)
        assertEquals("""{"relation":"direct","relation_id":"c1","device_id":null,"source":null,"control":false}""", direct.toJson())
        for (bad in listOf("""{"relation":"room","relation_id":"r"}""", """{"relation":"x","relation_id":"r"}""", """{"relation":"direct"}""", "no json", "[]")) {
            assertTrue(bad, ScreenTargetJson.parse(bad).isFailure)
        }
        assertEquals("Das Raumgerät fehlt im Bildschirmziel", ScreenTargetJson.parse("""{"relation":"room","relation_id":"r"}""").exceptionOrNull()?.message)
    }

    @Test
    fun zoom_runs_from_1_to_8_and_the_point_under_the_finger_stays() {
        val transform = ViewTransform()
        // A 1000×500 view with a picture of the same shape.
        transform.zoomBy(100f, 250f, 250f, 1000f, 500f, 2f)
        assertEquals(ViewTransform.MAX_ZOOM, transform.zoom, 0f)
        transform.zoomBy(0.001f, 250f, 250f, 1000f, 500f, 2f)
        assertEquals(1f, transform.zoom, 0f)
        assertEquals(0f, transform.panX, 0f)
        // Zooming 2× around the upper left corner keeps that corner of the picture in view.
        transform.zoomBy(2f, 0f, 0f, 1000f, 500f, 2f)
        assertEquals(500f, transform.panX, 0.01f)
        val (x, y) = transform.toSource(0f, 0f, 1000f, 500f, 2f)
        assertEquals(0, x)
        assertEquals(0, y)
        // The picture cannot be pulled past its edge, and a double tap fits it again.
        transform.panBy(10_000f, 10_000f, 1000f, 500f, 2f)
        assertEquals(500f, transform.panX, 0.01f)
        assertEquals(250f, transform.panY, 0.01f)
        transform.reset()
        assertEquals(32767.5, transform.toSource(500f, 250f, 1000f, 500f, 2f).first.toDouble(), 1.0)
        assertEquals(65535, transform.toSource(5000f, 250f, 1000f, 500f, 2f).first)
    }
}
