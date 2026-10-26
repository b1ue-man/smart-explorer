//! Public jnigraphics ABI; the only pixel borrow is bounded by one synchronous lock.
use jni::{
    objects::{JObject, Reference},
    sys::{jobject, JNIEnv},
    Env,
};
use smart_explorer::mobile::LoginBitmapTicket;
use std::ffi::c_void;
#[repr(C)]
#[derive(Default)]
struct BitmapInfo {
    width: u32,
    height: u32,
    stride: u32,
    format: i32,
    flags: u32,
}
#[link(name = "jnigraphics")]
extern "C" {
    fn AndroidBitmap_getInfo(env: *mut JNIEnv, bitmap: jobject, info: *mut BitmapInfo) -> i32;
    fn AndroidBitmap_lockPixels(env: *mut JNIEnv, bitmap: jobject, pixels: *mut *mut c_void)
        -> i32;
    fn AndroidBitmap_unlockPixels(env: *mut JNIEnv, bitmap: jobject) -> i32;
}
struct PixelLock {
    env: *mut JNIEnv,
    bitmap: jobject,
    locked: bool,
}
impl PixelLock {
    fn unlock(&mut self) -> bool {
        if !self.locked {
            return true;
        }
        // SAFETY: the local/global bitmap and JNI frame outlive this synchronous lock.
        let ok = unsafe { AndroidBitmap_unlockPixels(self.env, self.bitmap) } == 0;
        if ok {
            self.locked = false;
        }
        ok
    }
}
impl Drop for PixelLock {
    fn drop(&mut self) {
        let _ = self.unlock();
    }
}
/// Error true means the destination might remain locked and must remain owned.
pub(super) fn fill(
    env: &mut Env<'_>,
    bitmap: &JObject<'_>,
    ticket: &LoginBitmapTicket,
) -> Result<u32, bool> {
    let mut info = BitmapInfo::default();
    // SAFETY: all ABI arguments are valid for this JNI call.
    if unsafe { AndroidBitmap_getInfo(env.get_raw(), bitmap.as_raw(), &mut info) } != 0 {
        return Err(false);
    }
    let row = ticket.width().checked_mul(4).ok_or(false)?;
    let length = (info.stride as usize)
        .checked_mul(info.height as usize)
        .filter(|n| *n <= isize::MAX as usize)
        .ok_or(false)?;
    if info.width != ticket.width()
        || info.height != ticket.height()
        || info.format != 1
        || info.flags & (1 << 31) != 0
        || info.stride < row
    {
        return Err(false);
    }
    let mut pixels = std::ptr::null_mut();
    if unsafe { AndroidBitmap_lockPixels(env.get_raw(), bitmap.as_raw(), &mut pixels) } != 0 {
        return Err(false);
    }
    let mut lock = PixelLock {
        env: env.get_raw(),
        bitmap: bitmap.as_raw(),
        locked: true,
    };
    let copied = if pixels.is_null() {
        false
    } else {
        // SAFETY: NDK guarantees stable writable storage until unlock. No pointer escapes.
        ticket
            .fill_rgba(
                unsafe { std::slice::from_raw_parts_mut(pixels.cast(), length) },
                info.stride as usize,
            )
            .is_ok()
    };
    let unlocked = lock.unlock();
    if !unlocked {
        // Preserve the locked flag in the registered owner; cleanup can retry.
        lock.locked = false;
        return Err(true);
    }
    if copied {
        Ok(info.stride)
    } else {
        Err(false)
    }
}
pub(super) fn unlock(env: &mut Env<'_>, bitmap: &JObject<'_>) -> bool {
    unsafe { AndroidBitmap_unlockPixels(env.get_raw(), bitmap.as_raw()) == 0 }
}
