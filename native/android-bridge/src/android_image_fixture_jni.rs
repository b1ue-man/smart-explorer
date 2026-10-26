//! This entire module and its separate debug Kotlin class are absent from release.
use jni::{
    jni_str,
    objects::{JByteBuffer, JObject, JString},
    strings::JNIString,
    sys::{jboolean, jint, jlong},
    Env, EnvUnowned, Outcome,
};
use smart_explorer::mobile;
fn checked<T>(
    env: &mut Env<'_>,
    operation: &str,
    result: Result<T, String>,
) -> jni::errors::Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            env.throw_new(
                jni_str!("java/lang/IllegalStateException"),
                JNIString::from(format!("{operation}: {error}")),
            )?;
            Err(jni::errors::Error::JavaException)
        }
    }
}
fn positive(value: jlong) -> Result<u64, String> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "positive fixture identifier/offer required".into())
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_captureOpen<
    'local,
>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    width: jint,
    height: jint,
    output_width: jint,
    output_height: jint,
    kind: jint,
    heap: jlong,
    decoder: jboolean,
) -> JString<'local> {
    match unowned
        .with_env(|env| -> jni::errors::Result<JString<'local>> {
            let result = (|| {
                let side = |value| {
                    u32::try_from(value).map_err(|_| "invalid fixture geometry/kind".to_string())
                };
                mobile::android_fixture_capture_open(
                    side(width)?,
                    side(height)?,
                    side(output_width)?,
                    side(output_height)?,
                    side(kind)?,
                    positive(heap)?,
                    decoder,
                )
            })();
            let text = checked(env, "captureOpen", result)?;
            JString::from_str(env, &text)
        })
        .into_outcome()
    {
        Outcome::Ok(value) => value,
        _ => JString::default(),
    }
}
macro_rules! capture_string {
    ($symbol:ident,$operation:literal,$function:ident) => {
        #[no_mangle]
        pub extern "system" fn $symbol<'local>(
            mut unowned: EnvUnowned<'local>,
            _this: JObject<'local>,
            session: jlong,
        ) -> JString<'local> {
            match unowned
                .with_env(|env| -> jni::errors::Result<JString<'local>> {
                    let text = checked(
                        env,
                        $operation,
                        positive(session).and_then(mobile::$function),
                    )?;
                    JString::from_str(env, &text)
                })
                .into_outcome()
            {
                Outcome::Ok(value) => value,
                _ => JString::default(),
            }
        }
    };
}
capture_string!(
    Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_captureState,
    "captureState",
    android_fixture_capture_state
);
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_captureConsumeStill<
    'local,
>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    session: jlong,
    generation: jlong,
) -> JString<'local> {
    match unowned
        .with_env(|env| -> jni::errors::Result<JString<'local>> {
            let text = checked(
                env,
                "captureConsumeStill",
                positive(session).and_then(|id| {
                    positive(generation).and_then(|generation| {
                        mobile::android_fixture_capture_consume_still(id, generation)
                    })
                }),
            )?;
            JString::from_str(env, &text)
        })
        .into_outcome()
    {
        Outcome::Ok(value) => value,
        _ => JString::default(),
    }
}
macro_rules! capture_action {
    ($symbol:ident,$operation:literal,$function:ident) => {
        #[no_mangle]
        pub extern "system" fn $symbol(
            mut unowned: EnvUnowned<'_>,
            _this: JObject<'_>,
            session: jlong,
        ) {
            let _ = unowned.with_env(|env| -> jni::errors::Result<()> {
                checked(
                    env,
                    $operation,
                    positive(session).and_then(mobile::$function),
                )
            });
        }
    };
}
capture_action!(
    Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_captureReleaseDecoderReader,
    "captureReleaseDecoderReader",
    android_fixture_capture_release_reader
);
capture_action!(
    Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_captureReleaseViewer,
    "captureReleaseViewer",
    android_fixture_capture_release_viewer
);
capture_action!(
    Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_captureRetire,
    "captureRetire",
    android_fixture_capture_retire
);
capture_action!(
    Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_captureFinish,
    "captureFinish",
    android_fixture_capture_finish
);
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_captureViewerPullBuffer<
    'local,
>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    session: jlong,
) -> JByteBuffer<'local> {
    match unowned
        .with_env(|env| -> jni::errors::Result<JByteBuffer<'local>> {
            let (address, length) = checked(
                env,
                "captureViewerPullBuffer",
                positive(session).and_then(mobile::android_fixture_capture_pull_address),
            )?;
            // SAFETY: the fixture retains this arena token until onEnded, reader.close
            // and relinquishFixedBuffer have removed every Kotlin view.
            unsafe { env.new_direct_byte_buffer(address as *mut u8, length) }
        })
        .into_outcome()
    {
        Outcome::Ok(value) => value,
        _ => JByteBuffer::default(),
    }
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_loginOpen(
    mut unowned: EnvUnowned<'_>,
    _this: JObject<'_>,
    owner: jlong,
    heap: jlong,
    width: jint,
    height: jint,
) -> jlong {
    match unowned
        .with_env(|env| -> jni::errors::Result<jlong> {
            let result = (|| {
                let width =
                    u32::try_from(width).map_err(|_| "invalid login fixture width".to_string())?;
                let height = u32::try_from(height)
                    .map_err(|_| "invalid login fixture height".to_string())?;
                mobile::android_fixture_login_open(positive(owner)?, positive(heap)?, width, height)
            })();
            Ok(checked(env, "loginOpen", result)? as jlong)
        })
        .into_outcome()
    {
        Outcome::Ok(value) => value,
        _ => 0,
    }
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_loginEnqueue(
    mut unowned: EnvUnowned<'_>,
    _this: JObject<'_>,
    session: jlong,
    frame: jlong,
    pattern: jint,
) {
    let _ = unowned.with_env(|env| -> jni::errors::Result<()> {
        let result = positive(session).and_then(|id| {
            positive(frame).and_then(|frame| {
                u32::try_from(pattern)
                    .map_err(|_| "invalid login pattern".into())
                    .and_then(|pattern| mobile::android_fixture_login_enqueue(id, frame, pattern))
            })
        });
        checked(env, "loginEnqueue", result)
    });
}
fn login_state(session: u64) -> Result<serde_json::Value, String> {
    let text = mobile::android_fixture_login_state(session)?;
    let mut state: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let owner = state["owner"]
        .as_u64()
        .ok_or_else(|| "login fixture owner missing".to_string())?;
    state["native"] = super::login_bitmap_owner::fixture_state(owner)?;
    Ok(state)
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_loginState<
    'local,
>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    session: jlong,
) -> JString<'local> {
    match unowned
        .with_env(|env| -> jni::errors::Result<JString<'local>> {
            let state = checked(env, "loginState", positive(session).and_then(login_state))?;
            JString::from_str(env, &state.to_string())
        })
        .into_outcome()
    {
        Outcome::Ok(value) => value,
        _ => JString::default(),
    }
}
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_AndroidImageFixtureBridge_loginFinish(
    mut unowned: EnvUnowned<'_>,
    _this: JObject<'_>,
    session: jlong,
) {
    let _ = unowned.with_env(|env| -> jni::errors::Result<()> {
        let result = positive(session).and_then(|id| {
            let state = login_state(id)?;
            if state["native"]["global_refs"].as_u64() != Some(0)
                || state["native"]["render_targets"].as_u64() != Some(0)
            {
                return Err("login native GlobalRef/render target owners remain".into());
            }
            mobile::android_fixture_login_finish(id)
        });
        checked(env, "loginFinish", result)
    });
}
