package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.BuildConfig
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * G4 update against the local test feed of the runner (`updateFeedUrl` from
 * `<filesDir>/test-overrides.json`, served over http://10.0.2.2): check, download into
 * `<cache>/update/` and the SHA-256 the feed publishes.
 */
@RunWith(AndroidJUnit4::class)
class UpdateTaskTest {
    @Test
    fun checkAndDownloadFromTheTestFeed() = coreTest {
        val expectedVersion = TaskArgs.get("seFeedVersion")
        val check = Api.obj("update.check")
        assertEquals(BuildConfig.VERSION_NAME, check.text("current"))
        assertEquals(expectedVersion, check.text("latest"))
        assertTrue("Update nicht als verfügbar gemeldet: $check", check.bool("available"))

        val download = Api.runTask("update.download", timeoutMs = 600_000).resultObj()
        assertEquals(expectedVersion, download.text("version"))
        val apk = File(download.text("path"))
        assertEquals(File(appContext.cacheDir, "update").canonicalPath, apk.parentFile!!.canonicalPath)
        assertTrue(apk.name.endsWith(".apk"))
        assertEquals(TaskArgs.get("seFeedSha256"), Fixture.sha256(apk))
    }
}
