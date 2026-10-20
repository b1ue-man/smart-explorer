//! Proxy file descriptors of the documents provider (DRV-M8, strand S2
//! C3): the Kotlin `ProxyFileCallback` of each descriptor another app holds
//! calls these functions on the file's own `HandlerThread`. Each returns a
//! negated `errno` on failure (Kotlin throws `ErrnoException` with it) and
//! never throws a Java exception; file content is never logged.
use jni::objects::{JByteArray, JObject, JString};
use jni::sys::{jint, jlong};
use jni::{Env, EnvUnowned, Outcome};

/// `-EIO`: what the app sees when the bridge itself failed.
const BRIDGE_ERROR: i64 = -5;
/// Largest single transfer the platform hands over (AppFuse uses 128 KiB;
/// a larger request is cut, which the callback contract allows).
const MAX_CHUNK: usize = 1 << 20;

/// Runs `work` with the environment; a JNI failure or panic is `-EIO`.
fn number<'local>(
    unowned: &mut EnvUnowned<'local>,
    name: &str,
    work: impl FnOnce(&mut Env<'local>) -> jni::errors::Result<i64>,
) -> i64 {
    let outcome = unowned
        .with_env(|env| -> jni::errors::Result<i64> {
            let result = work(&mut *env);
            if env.exception_check() {
                env.exception_clear();
            }
            result
        })
        .into_outcome();
    match outcome {
        Outcome::Ok(value) => value,
        Outcome::Err(error) => {
            super::log_error(&format!("{name}: {error}"));
            BRIDGE_ERROR
        }
        Outcome::Panic(_) => {
            super::log_error(&format!("{name}: Panik in der JNI-Brücke"));
            BRIDGE_ERROR
        }
    }
}

fn narrow(value: i64) -> jint {
    jint::try_from(value).unwrap_or(BRIDGE_ERROR as jint)
}

/// The byte count of a request, at most `MAX_CHUNK` and the array length.
fn request_len(env: &Env<'_>, array: &JByteArray<'_>, size: jint) -> jni::errors::Result<usize> {
    let length = array.len(env)?;
    Ok(usize::try_from(size)
        .unwrap_or(0)
        .min(length)
        .min(MAX_CHUNK))
}

/// `external fun docOpen(documentId: String, mode: String): Long` — the
/// descriptor number (≥ 1) or `-errno`.
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_docOpen<'local>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    document: JString<'local>,
    mode: JString<'local>,
) -> jlong {
    number(&mut unowned, "docOpen", |env| {
        let document = document.try_to_string(env)?;
        let mode = mode.try_to_string(env)?;
        Ok(smart_explorer::mobile::doc_open(&document, &mode))
    })
}

/// `external fun docRead(handle: Long, offset: Long, size: Int, data:
/// ByteArray): Int` — bytes read into `data` (0 at the end) or `-errno`.
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_docRead<'local>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    handle: jlong,
    offset: jlong,
    size: jint,
    data: JByteArray<'local>,
) -> jint {
    narrow(number(&mut unowned, "docRead", |env| {
        let length = request_len(env, &data, size)?;
        let mut buffer = vec![0u8; length];
        let read = smart_explorer::mobile::doc_read(handle, offset, &mut buffer);
        if read > 0 {
            let count = usize::try_from(read).unwrap_or(0).min(length);
            let signed: Vec<i8> = buffer[..count].iter().map(|byte| *byte as i8).collect();
            data.set_region(env, 0, &signed)?;
        }
        Ok(i64::from(read))
    }))
}

/// `external fun docWrite(handle: Long, offset: Long, size: Int, data:
/// ByteArray): Int` — bytes written or `-errno`.
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_docWrite<'local>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    handle: jlong,
    offset: jlong,
    size: jint,
    data: JByteArray<'local>,
) -> jint {
    narrow(number(&mut unowned, "docWrite", |env| {
        let length = request_len(env, &data, size)?;
        let mut signed = vec![0i8; length];
        data.get_region(env, 0, &mut signed)?;
        let bytes: Vec<u8> = signed.iter().map(|byte| *byte as u8).collect();
        Ok(i64::from(smart_explorer::mobile::doc_write(
            handle, offset, &bytes,
        )))
    }))
}

/// `external fun docSize(handle: Long): Long` — from the handle, without
/// the network; `-errno` on failure.
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_docSize<'local>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    handle: jlong,
) -> jlong {
    number(&mut unowned, "docSize", |_| {
        Ok(smart_explorer::mobile::doc_size(handle))
    })
}

/// `external fun docFsync(handle: Long): Int` — publishes now; 0 or
/// `-errno`.
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_docFsync<'local>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    handle: jlong,
) -> jint {
    narrow(number(&mut unowned, "docFsync", |_| {
        Ok(i64::from(smart_explorer::mobile::doc_fsync(handle)))
    }))
}

/// `external fun docRelease(handle: Long): Int` — never blocks; 0 or
/// `-errno`.
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_docRelease<'local>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    handle: jlong,
) -> jint {
    narrow(number(&mut unowned, "docRelease", |_| {
        Ok(i64::from(smart_explorer::mobile::doc_release(handle)))
    }))
}
