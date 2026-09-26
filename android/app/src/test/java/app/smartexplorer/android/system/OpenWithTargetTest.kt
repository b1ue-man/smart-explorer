package app.smartexplorer.android.system

import app.smartexplorer.android.core.VolumeInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class OpenWithTargetTest {
    private val volumes = listOf(
        VolumeInfo("/storage/emulated/0", "Intern", primary = true, removable = false),
        VolumeInfo("/storage/1A2B-3C4D", "SD-Karte", primary = false, removable = true),
    )

    @Test
    fun documentIdsMapToTheirVolume() {
        assertEquals("/storage/emulated/0/Pictures", OpenWithTarget.pathForDocumentId("primary:Pictures", volumes))
        assertEquals("/storage/emulated/0", OpenWithTarget.pathForDocumentId("primary:", volumes))
        assertEquals("/storage/1A2B-3C4D/DCIM/Camera", OpenWithTarget.pathForDocumentId("1A2B-3C4D:DCIM/Camera", volumes))
        // Unknown root, missing separator.
        assertNull(OpenWithTarget.pathForDocumentId("9999-0000:DCIM", volumes))
        assertNull(OpenWithTarget.pathForDocumentId("primary", volumes))
    }

    @Test
    fun onlyPathsOnReportedVolumesOutsidePrivateAreasAreAccepted() {
        assertEquals("/storage/emulated/0/Download", OpenWithTarget.acceptedPath("/storage/emulated/0//Download/", volumes))
        assertEquals("/storage/1A2B-3C4D", OpenWithTarget.acceptedPath("/storage/1A2B-3C4D", volumes))
        assertEquals("/storage/emulated/0/Android/data", OpenWithTarget.acceptedPath("/storage/emulated/0/Android/data", volumes))
        // Other apps' private areas, app-internal storage, traversal, relative paths.
        assertNull(OpenWithTarget.acceptedPath("/storage/emulated/0/Android/data/com.example/files", volumes))
        assertNull(OpenWithTarget.acceptedPath("/storage/1A2B-3C4D/android/OBB/com.game", volumes))
        assertNull(OpenWithTarget.acceptedPath("/data/user/0/app.smartexplorer.android/files", volumes))
        assertNull(OpenWithTarget.acceptedPath("/storage/emulated/0/../../data", volumes))
        assertNull(OpenWithTarget.acceptedPath("storage/emulated/0/Download", volumes))
        // A sibling whose name only starts like a volume is not on it.
        assertNull(OpenWithTarget.acceptedPath("/storage/emulated/00/x", volumes))
    }
}
