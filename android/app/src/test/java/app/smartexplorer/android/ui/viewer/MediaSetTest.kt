package app.smartexplorer.android.ui.viewer

import app.smartexplorer.android.core.Entry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class MediaSetTest {
    private fun entry(name: String, kind: String, isDir: Boolean = false) =
        Entry(name = name, location = "/storage/emulated/0/DCIM/$name", isDir = isDir, kind = kind)

    private val shown = listOf(
        entry("Camera", "dir", isDir = true),
        entry("a.jpg", "image"),
        entry("Notiz.txt", "text"),
        entry("b.mp4", "video"),
        entry("Lied.mp3", "audio"),
        entry("c.png", "image"),
        entry("Ton.m4a", "audio"),
    )

    @Test
    fun picturesAndVideosAreBrowsedTogetherInListOrder() {
        val set = mediaSetFor(shown, shown[3], local = true)!!
        assertEquals(listOf("a.jpg", "b.mp4", "c.png"), set.items.map { it.name })
        assertEquals(1, set.start)
        assertEquals(true, set.local)
        assertEquals(2, mediaSetFor(shown, shown[5], local = false)!!.start)
    }

    @Test
    fun audioIsBrowsedSeparately() {
        val set = mediaSetFor(shown, shown[6], local = false)!!
        assertEquals(listOf("Lied.mp3", "Ton.m4a"), set.items.map { it.name })
        assertEquals(1, set.start)
        assertEquals(false, set.local)
    }

    @Test
    fun foldersAndOtherFilesOpenWithoutViewer() {
        assertNull(mediaSetFor(shown, shown[0], local = true))
        assertNull(mediaSetFor(shown, shown[2], local = true))
        assertNull(mediaSetFor(shown, entry("x.jpg", "image", isDir = true), local = true))
    }

    @Test
    fun aMediumOutsideTheShownListIsViewedAlone() {
        val single = entry("allein.webp", "image")
        val set = mediaSetFor(shown, single, local = true)!!
        assertEquals(listOf(single), set.items)
        assertEquals(0, set.start)
        assertEquals(MediaGroup.Visual, mediaGroupOf("video"))
        assertEquals(MediaGroup.Audio, mediaGroupOf("audio"))
        assertNull(mediaGroupOf("document"))
    }

    @Test
    fun playerClockShowsMinutesAndHours() {
        assertEquals("0:00", clock(0))
        assertEquals("1:05", clock(65_400))
        assertEquals("1:01:01", clock(3_661_000))
        assertEquals("0:00", clock(-5))
    }
}
