//! Immediate login control; independent of bitmap and renderer ownership.
use jni::{EnvUnowned, objects::JObject, sys::jlong};
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
