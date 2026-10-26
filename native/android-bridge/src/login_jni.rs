//! Bounded direct-buffer login pixels and immediate owner cancellation.
use jni::{EnvUnowned, Outcome, objects::{JByteBuffer, JObject}, sys::{jint, jlong}};
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_loginPull<'local>(mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>, session: jlong, dst: JByteBuffer<'local>, timeout_ms: jint) -> jint {
    let result = unowned.with_env(|env| -> jni::errors::Result<jint> {
        let Ok(id) = u64::try_from(session) else { return Ok(-4); };
        let address = env.get_direct_buffer_address(&dst)?; let capacity = env.get_direct_buffer_capacity(&dst)?;
        if address.is_null() || capacity > isize::MAX as usize { return Ok(-4); }
        // The direct buffer is exclusively handed to this call; the slice cannot escape.
        let bytes = unsafe { std::slice::from_raw_parts_mut(address, capacity) };
        let wait = std::time::Duration::from_millis(u64::try_from(timeout_ms).unwrap_or(0).min(1000));
        Ok(smart_explorer::mobile::login_pull(id, bytes, wait))
    }).into_outcome();
    match result { Outcome::Ok(value) => value, Outcome::Err(_) | Outcome::Panic(_) => -4 }
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_loginClose<'local>(mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>, session: jlong) {
    let _ = unowned.with_env(|_| -> jni::errors::Result<()> {
        if let Ok(id) = u64::try_from(session) { smart_explorer::mobile::login_close(id); } Ok(())
    }).into_outcome();
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_loginRelease<'local>(mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>, session: jlong) {
    let _ = unowned.with_env(|_| -> jni::errors::Result<()> {
        if let Ok(id) = u64::try_from(session) { smart_explorer::mobile::login_release(id); } Ok(())
    }).into_outcome();
}
