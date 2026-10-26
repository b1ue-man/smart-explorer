package app.smartexplorer.android.ui.login

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import app.smartexplorer.android.api.LoginTarget
import app.smartexplorer.android.api.LoginToken
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class LoginBitmapOwnerTest {
    private class Calls(val bitmap: Bitmap) : LoginBitmapCalls {
        var borrows = 0
        var retiring = false
        val ended = CountDownLatch(1)
        @Synchronized override fun borrow(image: LoginPictureDescriptor): Bitmap? {
            if (retiring) return null
            borrows++; return bitmap
        }
        @Synchronized override fun endBorrow(image: LoginPictureDescriptor) { borrows--; finish() }
        @Synchronized override fun retire(image: LoginPictureDescriptor) { retiring = true; finish() }
        private fun finish() { if (retiring && borrows == 0) { bitmap.recycle(); ended.countDown() } }
        override fun claimTarget(owner: Long, width: Int, height: Int): Long = error("software draw must not create a render target")
        override fun releaseTarget(owner: Long, target: Long): Unit = error("software draw has no render target")
    }
    private fun picture(calls: Calls) = LoginPicture(LoginBitmapOwner(LoginPictureDescriptor(1,2,3,LoginToken(4,2,2,"a".repeat(64))),calls))

    @Test fun software_raster_borrow_survives_state_owner_exchange_until_draw_returns() {
        val source = Bitmap.createBitmap(2,2,Bitmap.Config.ARGB_8888).apply { eraseColor(Color.RED) }
        val calls = Calls(source)
        val image = picture(calls)
        val draw = image.borrow()!!
        image.close()
        LoginImagePane().worker.submit {}.get(5,TimeUnit.SECONDS)
        assertFalse(source.isRecycled)
        val output = Bitmap.createBitmap(4,4,Bitmap.Config.ARGB_8888)
        draw.draw(Canvas(output),RectF(0f,0f,4f,4f),Paint(Paint.FILTER_BITMAP_FLAG))
        assertEquals(Color.RED,output.getPixel(2,2))
        draw.close()
        assertTrue(calls.ended.await(5,TimeUnit.SECONDS))
        assertTrue(source.isRecycled)
        output.recycle()
    }
    @Test fun view_reference_keeps_source_after_latest_state_replacement() {
        val source = Bitmap.createBitmap(2,2,Bitmap.Config.ARGB_8888)
        val calls = Calls(source)
        val state = picture(calls)
        val view = state.retain()!!
        state.close()
        LoginImagePane().worker.submit {}.get(5,TimeUnit.SECONDS)
        assertFalse(source.isRecycled)
        view.close()
        assertTrue(calls.ended.await(5,TimeUnit.SECONDS))
        assertNull(state.retain())
    }
    @Test fun delayed_presented_callback_cannot_reauthorize_after_release_focus_change_or_close() {
        val session = LoginSession(LoginTarget(JsonObject(emptyMap())),"fixture-user",true,LoginImagePane())
        val token = LoginToken(1,2,2,"b".repeat(64))
        val released = session.presentationStamp()
        session.release(); session.presented(token,released)
        assertNull(session.drawnToken())
        val focus = session.presentationStamp()
        session.focus(true,true); session.presented(token,focus)
        assertNull(session.drawnToken())
        val closed = session.presentationStamp()
        session.close(); session.presented(token,closed)
        assertNull(session.drawnToken())
        assertFalse(session.state.value.input_ready)
    }
}
