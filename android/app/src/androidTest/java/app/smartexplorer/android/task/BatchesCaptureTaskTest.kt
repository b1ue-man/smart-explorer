package app.smartexplorer.android.task

import androidx.test.ext.junit.runners.AndroidJUnit4
import app.smartexplorer.android.core.NativeBridge
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith

/** JNI rejection cases; the native C9 fixtures exercise admitted ownership and resize. */
@RunWith(AndroidJUnit4::class)
class BatchesCaptureTaskTest {
    @Test fun unknownSessionCannotObtainPixelsOrExternalResourceCredit() {
        val unknown=Long.MAX_VALUE
        assertEquals(0L,NativeBridge.screenCaptureBegin(unknown,1920,1080,2))
        assertEquals(0L,NativeBridge.screenCaptureClaim(unknown,1,2,1920L*1080*4,true))
        assertEquals(0L,NativeBridge.screenCaptureClaim(unknown,1,0,1920L*1080*4*3,false))
        assertNull(NativeBridge.screenCaptureBuffer(0))
        NativeBridge.screenCaptureRelease(0)
    }
}
