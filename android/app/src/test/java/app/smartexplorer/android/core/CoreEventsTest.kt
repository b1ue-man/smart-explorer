package app.smartexplorer.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** `pollEvents` envelopes (api.md §3) through the app's own event decoder and task store. */
class CoreEventsTest {
    @Test
    fun everyEventTypeDecodes() {
        val raw = """{"ok":[
            {"type":"task","task":{"id":"t1","kind":"transfer","title":"Kopieren","state":"running","doneBytes":5,"totalBytes":10}},
            {"type":"share"},
            {"type":"shareRequest","count":2},
            {"type":"edits"},
            {"type":"jobs"},
            {"type":"openUrl","url":"https://accounts.google.com/o/oauth2/auth"},
            {"type":"error","action":"Kopieren","message":"Kein Platz"},
            {"type":"volumes"}
        ]}"""
        val events = CoreEvents.parse(Core.json, raw)!!
        assertEquals(8, events.size)
        val task = events[0] as CoreEvent.Task
        assertEquals("t1", task.task.id)
        assertEquals(5L, task.task.doneBytes)
        assertEquals(CoreEvent.Share, events[1])
        assertEquals(CoreEvent.ShareRequest(2), events[2])
        assertEquals(CoreEvent.Edits, events[3])
        assertEquals(CoreEvent.Jobs, events[4])
        assertEquals(CoreEvent.OpenUrl("https://accounts.google.com/o/oauth2/auth"), events[5])
        assertEquals(CoreEvent.Error("Kopieren", "Kein Platz"), events[6])
        assertEquals(CoreEvent.Volumes, events[7])
    }

    @Test
    fun emptyPollAndUnknownTypes() {
        assertTrue(CoreEvents.parse(Core.json, """{"ok":[]}""")!!.isEmpty())
        // Unknown event types are skipped without failing the batch.
        val events = CoreEvents.parse(Core.json, """{"ok":[{"type":"future"},{"type":"jobs"}]}""")!!
        assertEquals(listOf<CoreEvent>(CoreEvent.Jobs), events)
    }

    @Test
    fun taskStoreKeepsFinishedStateSeenByEvents() {
        val store = TaskStore()
        store.upsert(TaskInfo(id = "a", kind = "transfer", title = "A", state = "done"))
        store.upsert(TaskInfo(id = "b", kind = "scan", title = "B", state = "running"))
        // A snapshot taken before "a" finished must not revive it; "b" (not in the snapshot) stays.
        store.replaceAll(listOf(TaskInfo(id = "a", kind = "transfer", title = "A", state = "running")))
        val byId = store.tasks.value.associateBy { it.id }
        assertEquals("done", byId.getValue("a").state)
        assertEquals("running", byId.getValue("b").state)
        store.upsert(TaskInfo(id = "b", kind = "scan", title = "B", state = "canceled"))
        assertEquals("canceled", store.tasks.value.first { it.id == "b" }.state)
    }
}
