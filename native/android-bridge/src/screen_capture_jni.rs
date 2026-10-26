//! Explicit generation and native-owned direct buffers for Android capture.
use jni::{
    objects::{JByteBuffer, JObject},
    sys::{jboolean, jint, jlong},
    EnvUnowned, Outcome,
};

#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_screenCaptureBegin(
    mut env: EnvUnowned<'_>,
    _this: JObject<'_>,
    session: jlong,
    width: jint,
    height: jint,
    kind: jint,
) -> jlong {
    match env
        .with_env(|_| -> jni::errors::Result<jlong> {
            let (Ok(session), Ok(width), Ok(height), Ok(kind)) = (
                u64::try_from(session),
                u32::try_from(width),
                u32::try_from(height),
                u32::try_from(kind),
            ) else {
                return Ok(0);
            };
            Ok(smart_explorer::mobile::screen_capture_begin(session, width, height, kind) as jlong)
        })
        .into_outcome()
    {
        Outcome::Ok(value) => value,
        _ => 0,
    }
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_screenCaptureClaim(
    mut env: EnvUnowned<'_>,
    _this: JObject<'_>,
    session: jlong,
    generation: jlong,
    owner: jint,
    bytes: jlong,
    buffer: jboolean,
) -> jlong {
    match env
        .with_env(|_| -> jni::errors::Result<jlong> {
            let (Ok(session), Ok(generation), Ok(owner), Ok(bytes)) = (
                u64::try_from(session),
                u64::try_from(generation),
                u32::try_from(owner),
                u64::try_from(bytes),
            ) else {
                return Ok(0);
            };
            Ok(smart_explorer::mobile::screen_capture_claim(
                session, generation, owner, bytes, buffer,
            ) as jlong)
        })
        .into_outcome()
    {
        Outcome::Ok(value) => value,
        _ => 0,
    }
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_screenCaptureBuffer<
    'local,
>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    token: jlong,
) -> JByteBuffer<'local> {
    match unowned
        .with_env(|env| -> jni::errors::Result<JByteBuffer<'local>> {
            let Some((address, length)) = u64::try_from(token)
                .ok()
                .and_then(smart_explorer::mobile::screen_capture_address)
            else {
                return Ok(JByteBuffer::default());
            };
            // SAFETY: the native arena owns this fixed allocation. The sole Kotlin
            // CaptureBuffer owner holds token through every access; duplicates are
            // scoped, and release runs only after its final reader exits.
            unsafe { env.new_direct_byte_buffer(address as *mut u8, length) }
        })
        .into_outcome()
    {
        Outcome::Ok(value) => value,
        _ => JByteBuffer::default(),
    }
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_screenCaptureRelease(
    mut env: EnvUnowned<'_>,
    _this: JObject<'_>,
    token: jlong,
) {
    let _ = env.with_env(|_| -> jni::errors::Result<()> {
        if let Ok(token) = u64::try_from(token) {
            smart_explorer::mobile::screen_capture_release(token);
        }
        Ok(())
    });
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_screenPushAdmitted<
    'local,
>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    session: jlong,
    generation: jlong,
    src: JByteBuffer<'local>,
    offset: jint,
    length: jint,
    pts_us: jlong,
    flags: jint,
) -> jint {
    match unowned
        .with_env(|env| -> jni::errors::Result<jint> {
            let (Ok(session), Ok(generation)) = (u64::try_from(session), u64::try_from(generation))
            else {
                return Ok(-3);
            };
            Ok(
                super::screen_jni::with_direct_bytes(env, &src, offset, length, |bytes| {
                    smart_explorer::mobile::screen_push_admitted(
                        session,
                        generation,
                        bytes,
                        u64::try_from(pts_us).unwrap_or(0),
                        flags as u32,
                    )
                })
                .unwrap_or(-3),
            )
        })
        .into_outcome()
    {
        Outcome::Ok(value) => value,
        _ => -3,
    }
}
