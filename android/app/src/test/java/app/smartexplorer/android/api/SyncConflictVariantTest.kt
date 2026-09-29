package app.smartexplorer.android.api

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SyncConflictVariantTest {
    @Test
    fun duplicateVersionsRetainTheirExactSelectionIds() {
        val result = Json.decodeFromString(SyncConflicts.serializer(), """
            {"available":true,"items":[{"cid":"c7","path":"Notebook/.obsidian/appearance.json",
              "a":{"exists":true,"size":4,"mtimeMs":123},
              "b":{"exists":true,"needsVariantChoice":true,"variants":[
                {"id":"drive-one","size":4,"mtimeMs":100,"checksum":"first"},
                {"id":"drive-two","size":8,"mtimeMs":200,"checksum":"second"}]},"text":false}]}
        """.trimIndent())
        val item = result.items.single()
        assertFalse(item.a!!.needsVariantChoice)
        assertTrue(item.b!!.needsVariantChoice)
        assertEquals("drive-two", item.b!!.variants[1].id)
        assertEquals(8L, item.b!!.variants[1].size)
        assertEquals(200L, item.b!!.variants[1].mtimeMs)
        assertEquals("second", item.b!!.variants[1].checksum)
        assertFalse(item.text)
    }

    @Test
    fun ordinaryConflictResponsesRemainCompatible() {
        val result = Json.decodeFromString(SyncConflict.serializer(), """
            {"cid":"c8","path":"note.txt","a":{"exists":false},
             "b":{"exists":true,"size":10,"mtimeMs":42},"text":false}
        """.trimIndent())
        assertFalse(result.a!!.exists)
        assertTrue(result.b!!.exists)
        assertFalse(result.b!!.needsVariantChoice)
        assertTrue(result.b!!.variants.isEmpty())
    }
}
