package app.smartexplorer.android.task

import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.RectF
import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.smartexplorer.android.api.LoginToken
import app.smartexplorer.android.ui.login.LoginBitmapCalls
import app.smartexplorer.android.ui.login.LoginBitmapOwner
import app.smartexplorer.android.ui.login.LoginBitmapRenderer
import app.smartexplorer.android.ui.login.LoginBitmapSurface
import app.smartexplorer.android.ui.login.LoginImagePane
import app.smartexplorer.android.ui.login.LoginPicture
import app.smartexplorer.android.ui.login.LoginPictureDescriptor
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Public HWUI/TextureView fixture with simulated native ownership; no JNI/NDK coverage. */
@RunWith(AndroidJUnit4::class)
class LoginImageTaskTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()
    private class Calls : LoginBitmapCalls {
        val bitmap = Bitmap.createBitmap(2,2,Bitmap.Config.ARGB_8888).apply { eraseColor(Color.RED) }
        var borrows = 0
        var retiring = false
        var targets = 0
        var next = 1L
        val ended = CountDownLatch(1)
        val targetEnded = CountDownLatch(1)
        @Synchronized override fun borrow(image: LoginPictureDescriptor): Bitmap? { if (retiring) return null; borrows++; return bitmap }
        @Synchronized override fun endBorrow(image: LoginPictureDescriptor) { borrows--; finish() }
        @Synchronized override fun retire(image: LoginPictureDescriptor) { retiring = true; finish() }
        private fun finish() { if (retiring && borrows == 0) { bitmap.recycle(); ended.countDown() } }
        @Synchronized override fun claimTarget(owner: Long, width: Int, height: Int): Long { if (targets == 2) return 0; targets++; return next++ }
        @Synchronized override fun releaseTarget(owner: Long, target: Long) { targets--; targetEnded.countDown() }
    }
    private fun onUi(action: () -> Unit) = InstrumentationRegistry.getInstrumentation().runOnMainSync(action)
    private class Fixture(val calls: Calls, val pane: LoginImagePane, val renderer: LoginBitmapRenderer, val surface: LoginBitmapSurface, val picture: LoginPicture)
    private fun rendered(): Fixture {
        val calls = Calls()
        val pane = LoginImagePane(calls,1)
        val renderer = LoginBitmapRenderer(pane)
        val picture = LoginPicture(LoginBitmapOwner(LoginPictureDescriptor(1,2,3,LoginToken(1,2,2,"a".repeat(64))),calls))
        val presented = CountDownLatch(1)
        var beforeProduction = 0L
        lateinit var surface: LoginBitmapSurface
        onUi {
            surface = LoginBitmapSurface(compose.activity,renderer) {
                beforeProduction = surface.surfaceTexture!!.timestamp
                surface.render(picture,RectF(0f,0f,surface.width.toFloat(),surface.height.toFloat())) { presented.countDown() }
            }
            compose.activity.setContentView(surface)
        }
        assertTrue("HWUI commit and TextureView update must agree",presented.await(15,TimeUnit.SECONDS))
        onUi {
            assertNotEquals("An attach/update callback must not stand in for an acquired buffer",
                beforeProduction, surface.surfaceTexture!!.timestamp)
        }
        return Fixture(calls,pane,renderer,surface,picture)
    }
    @Test fun close_without_followup_draw_waits_for_public_destroy_off_ui() {
        val fixture = rendered()
        val entered = CountDownLatch(1)
        val continueRetirement = CountDownLatch(1)
        fixture.pane.worker.execute { entered.countDown(); check(continueRetirement.await(15,TimeUnit.SECONDS)) }
        assertTrue(entered.await(5,TimeUnit.SECONDS))
        val requested = AtomicBoolean(false)
        try {
            onUi { fixture.picture.close(); fixture.renderer.close(); requested.set(true) }
            assertTrue(requested.get())
            assertFalse(fixture.calls.bitmap.isRecycled)
        } finally { continueRetirement.countDown() }
        assertTrue(fixture.calls.ended.await(15,TimeUnit.SECONDS))
        assertTrue(fixture.calls.targetEnded.await(15,TimeUnit.SECONDS))
        assertTrue(fixture.calls.bitmap.isRecycled)
    }
    @Test fun texture_detach_transfers_release_responsibility_until_renderer_ends() {
        val fixture = rendered()
        val texture = fixture.surface.surfaceTexture!!
        fixture.picture.close()
        onUi { compose.activity.setContentView(View(compose.activity)) }
        assertTrue(fixture.calls.ended.await(15,TimeUnit.SECONDS))
        assertTrue(fixture.calls.targetEnded.await(15,TimeUnit.SECONDS))
        assertTrue(texture.isReleased)
        assertTrue(fixture.calls.bitmap.isRecycled)
    }
}
