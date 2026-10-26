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
import app.smartexplorer.android.core.AndroidImageFixtureBridge as ImageBridge
import app.smartexplorer.android.core.NativeBridge
import app.smartexplorer.android.screen.ScreenFrameLimits
import android.os.SystemClock
import kotlinx.serialization.json.Json
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

/** Existing simulated-native HWUI cases plus real ViewerFrame/JNI/NDK ownership cases. */
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
    private class NativeFixture(val pane:LoginImagePane,val session:Long) {
        fun state()=Json.parseToJsonElement(ImageBridge.loginState(session)).obj()
        fun enqueue(frame:Long) = ImageBridge.loginEnqueue(session,frame,0)
        fun take():LoginPicture = checkNotNull(pane.take(session)) {"Real ViewerFrame not taken"}
        fun awaitEnded() {
            val deadline=SystemClock.uptimeMillis()+15_000
            var observed=state()
            while((observed.long("live_bytes")!=0L || observed.field("native").obj().long("global_refs")!=0L ||
                    observed.field("native").obj().long("render_targets")!=0L) && SystemClock.uptimeMillis()<deadline) {
                SystemClock.sleep(10);observed=state()
            }
            TaskReport.note("login.jni.retirement",observed.toString())
            assertEquals("Real image claims remain: $observed",0L,observed.long("live_bytes"))
            assertEquals(0L,observed.field("native").obj().long("global_refs"))
            assertEquals(0L,observed.field("native").obj().long("render_targets"))
            ImageBridge.loginFinish(session)
        }
    }
    private suspend fun nativeFixture():NativeFixture {
        val pane=LoginImagePane()
        try {
            val owner=pane.openHandle()
            val session=ImageBridge.loginOpen(owner,ScreenFrameLimits.workingBytes(),3,4)
            check(session>0)
            return NativeFixture(pane,session)
        } catch(failure:Throwable) {pane.close();throw failure}
    }
    /** This reference is used for pixels only inside the explicit native borrow. */
    private fun realBitmap(fixture:NativeFixture,picture:LoginPicture):Bitmap {
        val image=picture.identity
        val bitmap=checkNotNull(NativeBridge.loginBitmapBorrow(image.owner,image.picture,image.generation))
        try {
            assertEquals(Bitmap.Config.ARGB_8888,bitmap.config)
            assertEquals(3,bitmap.width);assertEquals(4,bitmap.height)
            assertTrue(bitmap.rowBytes>=bitmap.width*4)
            assertTrue(bitmap.allocationByteCount>=bitmap.rowBytes*bitmap.height)
            for(y in 0 until bitmap.height) for(x in 0 until bitmap.width) {
                val r=47+(x*29+y*13)%200
                val g=31+(x*11+y*23)%200
                val b=17+(x*19+y*7)%200
                assertEquals("NDK BGRA conversion/row address $x,$y",Color.rgb(r,g,b),bitmap.getPixel(x,y))
            }
            TaskReport.note("login.jni.bitmap","owner=${image.owner} picture=${image.picture} generation=${image.generation} rowBytes=${bitmap.rowBytes} allocation=${bitmap.allocationByteCount} state=${fixture.state()}")
            assertFalse(fixture.state().field("native").obj().objects("bitmap_owners").single().bool("locked"))
        } finally {NativeBridge.loginBitmapEndBorrow(image.owner,image.picture,image.generation)}
        return bitmap // No pixel access after endBorrow; only isRecycled observations.
    }
    private class NativeRendered(val renderer:LoginBitmapRenderer,val surface:LoginBitmapSurface)
    private fun renderedNative(fixture:NativeFixture,picture:LoginPicture):NativeRendered {
        val renderer=LoginBitmapRenderer(fixture.pane)
        val presented=CountDownLatch(1)
        lateinit var surface:LoginBitmapSurface
        var previousTimestamp=0L
        onUi {
            surface=LoginBitmapSurface(compose.activity,renderer) {
                previousTimestamp=surface.surfaceTexture!!.timestamp
                surface.render(picture,RectF(0f,0f,surface.width.toFloat(),surface.height.toFloat())) {presented.countDown()}
            }
            compose.activity.setContentView(surface)
        }
        try {
            assertTrue("Actual JNI bitmap HWUI commit and acquired TextureView buffer missing",presented.await(15,TimeUnit.SECONDS))
            onUi {assertNotEquals(previousTimestamp,surface.surfaceTexture!!.timestamp)}
            return NativeRendered(renderer,surface)
        } catch(failure:Throwable) {onUi {renderer.close()};throw failure}
    }
    @Test fun viewer_frame_jni_ndk_bitmap_retires_after_hwui_destroy() = coreTest {
        val fixture=nativeFixture()
        var picture:LoginPicture?=null
        var rendered:NativeRendered?=null
        val continueRetirement=CountDownLatch(1)
        try {
            fixture.enqueue(1)
            val actual=fixture.take();picture=actual
            val bitmap=realBitmap(fixture,actual)
            val drawn=renderedNative(fixture,actual);rendered=drawn
            val entered=CountDownLatch(1)
            fixture.pane.worker.execute {entered.countDown();check(continueRetirement.await(15,TimeUnit.SECONDS))}
            assertTrue(entered.await(5,TimeUnit.SECONDS))
            onUi {actual.close();drawn.renderer.close();fixture.pane.close()}
            val held=fixture.state()
            assertEquals(1L,held.long("bitmap_count"));assertEquals(1L,held.long("target_count"))
            assertTrue(held.long("live_bytes")>0)
            assertEquals(1L,held.field("native").obj().long("global_refs"))
            assertTrue(held.field("native").obj().objects("bitmap_owners").single().int("borrows")>0)
            assertFalse(bitmap.isRecycled)
            continueRetirement.countDown()
            fixture.awaitEnded()
            assertTrue(bitmap.isRecycled)
        } finally {
            continueRetirement.countDown()
            onUi {picture?.close();rendered?.renderer?.close();fixture.pane.close();compose.activity.setContentView(View(compose.activity))}
        }
    }
    @Test fun viewer_frame_jni_texture_detach_keeps_owner_until_renderer_ends() = coreTest {
        val fixture=nativeFixture()
        var picture:LoginPicture?=null
        var rendered:NativeRendered?=null
        val continueRetirement=CountDownLatch(1)
        try {
            fixture.enqueue(1)
            val actual=fixture.take();picture=actual
            val bitmap=realBitmap(fixture,actual)
            val drawn=renderedNative(fixture,actual);rendered=drawn
            val texture=drawn.surface.surfaceTexture!!
            val entered=CountDownLatch(1)
            fixture.pane.worker.execute {entered.countDown();check(continueRetirement.await(15,TimeUnit.SECONDS))}
            assertTrue(entered.await(5,TimeUnit.SECONDS))
            onUi {actual.close();compose.activity.setContentView(View(compose.activity));fixture.pane.close()}
            assertFalse("Detached texture released before renderer end",texture.isReleased)
            assertFalse(bitmap.isRecycled)
            assertEquals(1L,fixture.state().long("target_count"))
            assertTrue(fixture.state().long("live_bytes")>0)
            continueRetirement.countDown()
            fixture.awaitEnded()
            assertTrue(texture.isReleased);assertTrue(bitmap.isRecycled)
        } finally {
            continueRetirement.countDown()
            onUi {picture?.close();rendered?.renderer?.close();fixture.pane.close();compose.activity.setContentView(View(compose.activity))}
        }
    }
    @Test fun viewer_frame_jni_bitmap_slots_wait_for_real_retirement() = coreTest {
        val fixture=nativeFixture()
        val pictures=ArrayList<LoginPicture>()
        var borrowed:LoginPictureDescriptor?=null
        val continueRetirement=CountDownLatch(1)
        try {
            fixture.enqueue(1);val first=fixture.take();pictures+=first
            fixture.enqueue(2);val second=fixture.take();pictures+=second
            val key=first.identity
            assertNull(NativeBridge.loginBitmapBorrow(key.owner,key.picture,key.generation+1))
            assertNull(NativeBridge.loginBitmapTake(fixture.session+1,key.owner))
            val bitmap=checkNotNull(NativeBridge.loginBitmapBorrow(key.owner,key.picture,key.generation));borrowed=key
            fixture.enqueue(3)
            assertNull("Third bitmap must wait without consuming ViewerFrame",fixture.pane.take(fixture.session))
            var held=fixture.state()
            assertEquals(2L,held.long("bitmap_count"));assertEquals(1L,held.long("raw_count"));assertTrue(held.bool("queued"))
            val entered=CountDownLatch(1)
            fixture.pane.worker.execute {entered.countDown();check(continueRetirement.await(15,TimeUnit.SECONDS))}
            assertTrue(entered.await(5,TimeUnit.SECONDS))
            first.close()
            assertFalse(bitmap.isRecycled)
            assertNull(fixture.pane.take(fixture.session))
            continueRetirement.countDown()
            // A barrier confirms the real retirement request, while the borrow still holds bytes.
            val retired=CountDownLatch(1);fixture.pane.worker.execute {retired.countDown()}
            assertTrue(retired.await(5,TimeUnit.SECONDS))
            held=fixture.state()
            assertTrue(held.field("native").obj().objects("bitmap_owners").any {it.bool("retiring") && it.int("borrows")==1})
            assertEquals(2L,held.long("bitmap_count"));assertFalse(bitmap.isRecycled)
            NativeBridge.loginBitmapEndBorrow(key.owner,key.picture,key.generation);borrowed=null
            assertTrue(bitmap.isRecycled)
            val third=fixture.take();pictures+=third
            assertEquals(3L,third.token.frame_id)
            assertEquals(2L,fixture.state().long("bitmap_count"))
        } finally {
            continueRetirement.countDown()
            borrowed?.let {NativeBridge.loginBitmapEndBorrow(it.owner,it.picture,it.generation)}
            pictures.forEach {it.close()};fixture.pane.close()
        }
        fixture.awaitEnded()
    }
    @Test fun viewer_frame_jni_discarded_descriptor_releases_native_owner() = coreTest {
        val fixture=nativeFixture()
        var raw:String?=null
        try {
            fixture.enqueue(1)
            val descriptor=checkNotNull(NativeBridge.loginBitmapTake(fixture.session,fixture.pane.handle));raw=descriptor
            val value=Json.parseToJsonElement(descriptor).obj()
            val owner=value.long("owner");val picture=value.long("picture");val generation=value.long("generation")
            val bitmap=checkNotNull(NativeBridge.loginBitmapBorrow(owner,picture,generation))
            var stride=0;var allocation=0
            try {assertFalse(bitmap.isRecycled);stride=bitmap.rowBytes;allocation=bitmap.allocationByteCount}
            finally {NativeBridge.loginBitmapEndBorrow(owner,picture,generation)}
            val held=fixture.state()
            assertEquals(1L,held.long("bitmap_count"));assertEquals(1L,held.long("metadata_count"))
            assertEquals(1L,held.field("native").obj().long("global_refs"))
            assertEquals(stride,value.int("stride"));assertEquals(allocation,value.int("allocation_bytes"))
            NativeBridge.loginBitmapDiscard(owner+1,descriptor)
            val foreign=Json.parseToJsonElement(descriptor).obj().withFields("owner" to owner+1).toString()
            NativeBridge.loginBitmapDiscard(owner,foreign)
            assertFalse(bitmap.isRecycled);assertEquals(1L,fixture.state().long("bitmap_count"))
            NativeBridge.loginBitmapDiscard(owner,descriptor);raw=null
            assertTrue(bitmap.isRecycled)
            assertEquals(0L,fixture.state().long("live_bytes"))
            assertEquals(0L,fixture.state().field("native").obj().long("global_refs"))
        } finally {raw?.let {NativeBridge.loginBitmapDiscard(fixture.pane.handle,it)};fixture.pane.close()}
        fixture.awaitEnded()
    }

}
