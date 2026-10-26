//! Strong bitmap owner. A retire request waits for the last explicit draw borrow.
use jni::{
    jni_sig, jni_str,
    objects::{Global, JObject},
    Env, JValue,
};
use smart_explorer::mobile::{LoginBitmapLease, LoginBitmapTicket, LoginRenderTarget};
use std::{collections::BTreeMap, sync::Mutex};
pub(super) type Key = (u64, u64, u64);
struct BitmapOwner {
    bitmap: Global<JObject<'static>>,
    lease: LoginBitmapLease,
    borrows: u32,
    retiring: bool,
    locked: bool,
}
static BITMAPS: Mutex<BTreeMap<Key, BitmapOwner>> = Mutex::new(BTreeMap::new());
static TARGETS: Mutex<BTreeMap<u64, (u64, LoginRenderTarget)>> = Mutex::new(BTreeMap::new());
static NEXT_TARGET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn clear_exception(env: &mut Env<'_>) {
    if env.exception_check() {
        env.exception_clear();
    }
}
fn recycle(env: &mut Env<'_>, bitmap: &JObject<'_>) -> bool {
    clear_exception(env);
    let result = env
        .call_method(bitmap, jni_str!("recycle"), jni_sig!("()V"), &[])
        .is_ok();
    clear_exception(env);
    result
}
pub(super) fn create(
    env: &mut Env<'_>,
    ticket: LoginBitmapTicket,
) -> jni::errors::Result<Option<String>> {
    if !ticket.is_current() {
        return Ok(None);
    }
    let config = env
        .get_static_field(
            jni_str!("android/graphics/Bitmap$Config"),
            jni_str!("ARGB_8888"),
            jni_sig!("Landroid/graphics/Bitmap$Config;"),
        )?
        .l()?;
    let bitmap = env
        .call_static_method(
            jni_str!("android/graphics/Bitmap"),
            jni_str!("createBitmap"),
            jni_sig!("(IILandroid/graphics/Bitmap$Config;)Landroid/graphics/Bitmap;"),
            &[
                JValue::Int(ticket.width() as i32),
                JValue::Int(ticket.height() as i32),
                JValue::Object(&config),
            ],
        )?
        .check_null()?
        .l()?;
    let global = match env.new_global_ref(&bitmap) {
        Ok(global) => global,
        Err(error) => {
            // If Java refuses cleanup there is no proven allocation end. Preserve
            // this bounded pane slot rather than turn a failed global ref into credit.
            if !recycle(env, &bitmap) {
                std::mem::forget(ticket);
            }
            return Err(error);
        }
    };
    let filled = super::login_bitmap_pixels::fill(env, &bitmap, &ticket);
    let allocation = env
        .call_method(
            &bitmap,
            jni_str!("getAllocationByteCount"),
            jni_sig!("()I"),
            &[],
        )
        .and_then(|value| value.i())
        .ok();
    let mut descriptor = serde_json::from_str::<serde_json::Value>(&ticket.descriptor()).ok();
    if let Some(value) = descriptor.as_mut() {
        value["stride"] = serde_json::json!(filled.as_ref().ok());
        value["allocation_bytes"] = serde_json::json!(allocation);
    }
    let lease = ticket.finish(); // raw image is held through the successful/failed unlock.
    let key = (lease.owner(), lease.picture(), lease.generation());
    let publish =
        filled.is_ok() && allocation.is_some() && descriptor.is_some() && lease.is_current();
    let owner = BitmapOwner {
        bitmap: global,
        lease,
        borrows: 0,
        retiring: !publish,
        locked: filled == Err(true),
    };
    let mut owners = match BITMAPS.lock() {
        Ok(owners) => owners,
        Err(_) => {
            if !owner.locked && recycle(env, owner.bitmap.as_obj()) {
                drop(owner);
            } else {
                std::mem::forget(owner);
            }
            return Ok(None);
        }
    };
    owners.insert(key, owner);
    drop(owners);
    if !publish {
        retire(env, key);
        return if filled.is_err() || allocation.is_none() {
            Err(jni::errors::Error::NullPtr(
                "login bitmap fill or allocation diagnostics failed",
            ))
        } else {
            Ok(None)
        };
    }
    Ok(descriptor.map(|value| value.to_string()))
}
pub(super) fn borrow<'local>(
    env: &mut Env<'local>,
    key: Key,
) -> jni::errors::Result<JObject<'local>> {
    let Ok(mut owners) = BITMAPS.lock() else {
        return Ok(JObject::default());
    };
    let Some(owner) = owners.get_mut(&key) else {
        return Ok(JObject::default());
    };
    if owner.retiring || owner.locked || !owner.lease.is_current() {
        return Ok(JObject::default());
    }
    let Some(count) = owner.borrows.checked_add(1) else {
        return Ok(JObject::default());
    };
    let bitmap = env.new_local_ref(owner.bitmap.as_obj())?;
    owner.borrows = count;
    Ok(bitmap)
}
pub(super) fn end_borrow(env: &mut Env<'_>, key: Key) {
    if let Ok(mut owners) = BITMAPS.lock() {
        if let Some(owner) = owners.get_mut(&key) {
            if owner.borrows != 0 {
                owner.borrows -= 1;
            }
        }
    }
    cleanup(env, key);
}
pub(super) fn retire(env: &mut Env<'_>, key: Key) {
    if let Ok(mut owners) = BITMAPS.lock() {
        if let Some(owner) = owners.get_mut(&key) {
            owner.retiring = true;
        }
    }
    cleanup(env, key);
}
pub(super) fn discard_descriptor(env: &mut Env<'_>, owner: u64, descriptor: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(descriptor) else {
        return;
    };
    if value["owner"].as_u64() != Some(owner) {
        return;
    }
    if let (Some(picture), Some(generation)) =
        (value["picture"].as_u64(), value["generation"].as_u64())
    {
        retire(env, (owner, picture, generation));
    }
}
fn cleanup(env: &mut Env<'_>, key: Key) {
    let Ok(mut owners) = BITMAPS.lock() else {
        return;
    };
    let Some(owner) = owners.get_mut(&key) else {
        return;
    };
    if !owner.retiring || owner.borrows != 0 {
        return;
    }
    if owner.locked {
        clear_exception(env);
        if !super::login_bitmap_pixels::unlock(env, owner.bitmap.as_obj()) {
            clear_exception(env);
            return;
        }
        owner.locked = false;
    }
    if !recycle(env, owner.bitmap.as_obj()) {
        return;
    }
    // Explicit order: recycle, GlobalRef destruction, then the pixel/metadata claims.
    if let Some(owner) = owners.remove(&key) {
        let BitmapOwner { bitmap, lease, .. } = owner;
        drop(bitmap);
        drop(lease);
    }
}
pub(super) fn target_claim(owner: u64, width: u32, height: u32) -> u64 {
    let Some(target) = smart_explorer::mobile::login_render_target_claim(owner, width, height)
    else {
        return 0;
    };
    let Ok(id) = NEXT_TARGET.fetch_update(
        std::sync::atomic::Ordering::SeqCst,
        std::sync::atomic::Ordering::SeqCst,
        |id| id.checked_add(1).filter(|id| *id <= i64::MAX as u64),
    ) else {
        return 0;
    };
    let Ok(mut targets) = TARGETS.lock() else {
        return 0;
    };
    targets.insert(id, (owner, target));
    id
}
pub(super) fn target_release(owner: u64, id: u64) {
    if let Ok(mut targets) = TARGETS.lock() {
        if targets.get(&id).is_some_and(|(bound, _)| *bound == owner) {
            targets.remove(&id);
        }
    }
}

/// Actual registry owners only; observing creates no GlobalRef or slice alias.
#[cfg(feature = "android-instrumentation-fixtures")]
pub(super) fn fixture_state(owner: u64) -> Result<serde_json::Value, String> {
    let bitmaps = BITMAPS.lock().map_err(|_| "bitmap registry unavailable")?;
    let targets = TARGETS
        .lock()
        .map_err(|_| "render target registry unavailable")?;
    let entries = bitmaps
        .iter()
        .filter(|((bound, _, _), _)| *bound == owner)
        .map(|((_, picture, generation), bitmap)| {
            serde_json::json!({"picture":picture,"generation":generation,"borrows":bitmap.borrows,
            "retiring":bitmap.retiring,"locked":bitmap.locked,"bytes":bitmap.lease.bytes()})
        })
        .collect::<Vec<_>>();
    let count = entries.len();
    Ok(
        serde_json::json!({"bitmap_owners":entries,"global_refs":count,
        "render_targets":targets.values().filter(|(bound,_)|*bound==owner).count()}),
    )
}
