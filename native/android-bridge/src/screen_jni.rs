//! Binary path of this phone as screen host (SCR-M8, C4): pictures of the
//! app's MediaCodec encoder or accessibility stills go to the core with
//! `screenPush` from a direct `ByteBuffer` (no copy into a Java array), and
//! the app takes the core's control messages with `screenControl`. Neither
//! call throws a Java exception; a picture's bytes are never logged.
use jni::objects::{JByteBuffer, JObject, JString};
use jni::sys::{jint, jlong};
use jni::{EnvUnowned, Outcome};
use std::time::Duration;

/// `screenPush` answer when the bridge itself failed (buffer unreadable).
const PUSH_BRIDGE_ERROR: jint = -3;

/// Runs `read` on the bytes `offset..offset + length` of the direct buffer
/// `src`; `None` when the buffer is not direct or the range does not fit.
fn with_direct_bytes<R>(
    env: &jni::Env<'_>,
    src: &JByteBuffer<'_>,
    offset: jint,
    length: jint,
    read: impl FnOnce(&[u8]) -> R,
) -> Option<R> {
    let start = usize::try_from(offset).ok()?;
    let length = usize::try_from(length).ok()?;
    let address = env.get_direct_buffer_address(src).ok()?;
    let capacity = env.get_direct_buffer_capacity(src).ok()?;
    if address.is_null() || start.checked_add(length)? > capacity {
        return None;
    }
    // SAFETY: the range lies within the buffer's capacity (checked above);
    // the app keeps the buffer alive and unwritten while this JNI call runs
    // (it hands the buffer over until the call returns), and the slice does
    // not leave `read`.
    let bytes = unsafe { std::slice::from_raw_parts(address.add(start).cast_const(), length) };
    Some(read(bytes))
}

/// `external fun screenPush(session: Long, src: ByteBuffer, offset: Int,
/// length: Int, ptsUs: Long, flags: Int): Int` — 0 taken, −1 unknown or
/// ended session, −2 empty or too large, −3 the buffer was not readable.
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_screenPush<'local>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    session: jlong,
    src: JByteBuffer<'local>,
    offset: jint,
    length: jint,
    pts_us: jlong,
    flags: jint,
) -> jint {
    let outcome = unowned
        .with_env(|env| -> jni::errors::Result<jint> {
            let Ok(session) = u64::try_from(session) else {
                return Ok(PUSH_BRIDGE_ERROR);
            };
            let pts = u64::try_from(pts_us).unwrap_or(0);
            let pushed = with_direct_bytes(env, &src, offset, length, |bytes| {
                smart_explorer::mobile::screen_push(session, bytes, pts, flags as u32)
            });
            Ok(pushed.unwrap_or(PUSH_BRIDGE_ERROR))
        })
        .into_outcome();
    match outcome {
        Outcome::Ok(code) => code,
        Outcome::Err(error) => {
            super::log_error(&format!("screenPush: {error}"));
            PUSH_BRIDGE_ERROR
        }
        Outcome::Panic(_) => {
            super::log_error("screenPush: Panik in der JNI-Brücke");
            PUSH_BRIDGE_ERROR
        }
    }
}

/// `external fun screenControl(session: Long, timeoutMs: Int): String?` —
/// the next control message (JSON) or `null` after the timeout.
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_screenControl<'local>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    session: jlong,
    timeout_ms: jint,
) -> JString<'local> {
    let outcome = unowned
        .with_env(|env| -> jni::errors::Result<JString<'local>> {
            let Ok(session) = u64::try_from(session) else {
                return Ok(JString::default());
            };
            let timeout = Duration::from_millis(u64::try_from(timeout_ms).unwrap_or(0));
            match smart_explorer::mobile::screen_control(session, timeout) {
                Some(message) => JString::from_str(env, message),
                None => Ok(JString::default()),
            }
        })
        .into_outcome();
    match outcome {
        Outcome::Ok(text) => text,
        Outcome::Err(error) => {
            super::log_error(&format!("screenControl: {error}"));
            JString::default()
        }
        Outcome::Panic(_) => {
            super::log_error("screenControl: Panik in der JNI-Brücke");
            JString::default()
        }
    }
}
