//! Opaque login picture transfer. These methods never export a whole pixel buffer.
use jni::{
    objects::{JObject, JString},
    sys::{jint, jlong},
    EnvUnowned, Outcome,
};
fn key(owner: jlong, picture: jlong, generation: jlong) -> Option<super::login_bitmap_owner::Key> {
    Some((
        u64::try_from(owner).ok()?,
        u64::try_from(picture).ok()?,
        u64::try_from(generation).ok()?,
    ))
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_loginImageOwnerCreate(
    mut env: EnvUnowned<'_>,
    _this: JObject<'_>,
    heap: jlong,
) -> jlong {
    match env
        .with_env(|_| -> jni::errors::Result<jlong> {
            Ok(u64::try_from(heap)
                .ok()
                .map_or(0, smart_explorer::mobile::login_image_owner_create)
                as jlong)
        })
        .into_outcome()
    {
        Outcome::Ok(id) => id,
        _ => 0,
    }
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_loginImageOwnerClose(
    mut env: EnvUnowned<'_>,
    _this: JObject<'_>,
    owner: jlong,
) {
    let _ = env.with_env(|_| -> jni::errors::Result<()> {
        if let Ok(owner) = u64::try_from(owner) {
            smart_explorer::mobile::login_image_owner_close(owner);
        }
        Ok(())
    });
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_loginBitmapTake<'local>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    session: jlong,
    owner: jlong,
) -> JString<'local> {
    match unowned
        .with_env(|env| -> jni::errors::Result<JString<'local>> {
            let (Ok(session), Ok(owner)) = (u64::try_from(session), u64::try_from(owner)) else {
                return JString::from_str(env, "");
            };
            let ticket = match smart_explorer::mobile::login_bitmap_take(session, owner) {
                Ok(Some(ticket)) => ticket,
                Ok(None) => return JString::from_str(env, ""),
                Err(error) => {
                    super::log_error(&error);
                    return Ok(JString::default());
                }
            };
            let text = super::login_bitmap_owner::create(env, ticket)?;
            let Some(text) = text else {
                return JString::from_str(env, "");
            };
            match JString::from_str(env, &text) {
                Ok(result) => Ok(result),
                Err(error) => {
                    // Descriptor creation failed after publication into the registry.
                    super::login_bitmap_owner::discard_descriptor(env, owner, &text);
                    Err(error)
                }
            }
        })
        .into_outcome()
    {
        Outcome::Ok(result) => result,
        _ => JString::default(),
    }
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_loginBitmapDiscard<
    'local,
>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    owner: jlong,
    descriptor: JString<'local>,
) {
    let _ = unowned.with_env(|env| -> jni::errors::Result<()> {
        if let Ok(owner) = u64::try_from(owner) {
            let descriptor = descriptor.try_to_string(env)?;
            super::login_bitmap_owner::discard_descriptor(env, owner, &descriptor);
        }
        Ok(())
    });
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_loginBitmapBorrow<
    'local,
>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    owner: jlong,
    picture: jlong,
    generation: jlong,
) -> JObject<'local> {
    match unowned
        .with_env(|env| -> jni::errors::Result<JObject<'local>> {
            match key(owner, picture, generation) {
                Some(key) => super::login_bitmap_owner::borrow(env, key),
                None => Ok(JObject::default()),
            }
        })
        .into_outcome()
    {
        Outcome::Ok(result) => result,
        _ => JObject::default(),
    }
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_loginBitmapEndBorrow(
    mut unowned: EnvUnowned<'_>,
    _this: JObject<'_>,
    owner: jlong,
    picture: jlong,
    generation: jlong,
) {
    let _ = unowned.with_env(|env| -> jni::errors::Result<()> {
        if let Some(key) = key(owner, picture, generation) {
            super::login_bitmap_owner::end_borrow(env, key);
        }
        Ok(())
    });
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_loginBitmapRetire(
    mut unowned: EnvUnowned<'_>,
    _this: JObject<'_>,
    owner: jlong,
    picture: jlong,
    generation: jlong,
) {
    let _ = unowned.with_env(|env| -> jni::errors::Result<()> {
        if let Some(key) = key(owner, picture, generation) {
            super::login_bitmap_owner::retire(env, key);
        }
        Ok(())
    });
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_loginRenderTargetClaim(
    mut env: EnvUnowned<'_>,
    _this: JObject<'_>,
    owner: jlong,
    width: jint,
    height: jint,
) -> jlong {
    match env
        .with_env(|_| -> jni::errors::Result<jlong> {
            let (Ok(owner), Ok(width), Ok(height)) = (
                u64::try_from(owner),
                u32::try_from(width),
                u32::try_from(height),
            ) else {
                return Ok(0);
            };
            Ok(super::login_bitmap_owner::target_claim(owner, width, height) as jlong)
        })
        .into_outcome()
    {
        Outcome::Ok(result) => result,
        _ => 0,
    }
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_loginRenderTargetRelease(
    mut env: EnvUnowned<'_>,
    _this: JObject<'_>,
    owner: jlong,
    target: jlong,
) {
    let _ = env.with_env(|_| -> jni::errors::Result<()> {
        if let (Ok(owner), Ok(target)) = (u64::try_from(owner), u64::try_from(target)) {
            super::login_bitmap_owner::target_release(owner, target);
        }
        Ok(())
    });
}
