//! JNI bridge for the Android app (`libsmart_explorer_android.so`).
//!
//! Three native methods of the Kotlin `object`
//! `app.smartexplorer.android.core.NativeBridge` forward UTF-8 JSON to
//! `smart_explorer::mobile` (api.md §1). They are instance methods (no
//! `@JvmStatic`), so the second parameter is the object itself. No call ever
//! throws a Java exception on purpose: failures come back as
//! `{"err":{"kind":"internal",…}}`, panics are caught, and bridge problems
//! are written to logcat. On every other target this library is empty.
#![cfg(target_os = "android")]

use jni::objects::{Global, JObject, JString, Reference};
use jni::sys::jint;
use jni::{Env, EnvUnowned, JavaVM, Outcome};
use std::ffi::{c_char, c_int, CString};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const LOG_TAG: &str = "SmartExplorer";
const ANDROID_LOG_ERROR: c_int = 6;

#[link(name = "log")]
extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// Writes one error line to logcat (liblog).
fn log_error(text: &str) {
    let (Ok(tag), Ok(text)) = (CString::new(LOG_TAG), CString::new(text.replace('\0', " "))) else {
        return;
    };
    // SAFETY: both pointers are valid NUL-terminated strings for the call.
    unsafe {
        __android_log_write(ANDROID_LOG_ERROR, tag.as_ptr(), text.as_ptr());
    }
}

fn err_json(message: &str) -> String {
    serde_json::json!({ "err": { "kind": "internal", "message": message } }).to_string()
}

/// Runs `work` with the JNI environment and returns its JSON as a Java
/// string. JNI failures become an `err` envelope where possible.
fn respond<'local>(
    unowned: &mut EnvUnowned<'local>,
    name: &str,
    work: impl FnOnce(&mut Env<'local>) -> jni::errors::Result<String>,
) -> JString<'local> {
    let outcome = unowned
        .with_env(|env| -> jni::errors::Result<JString<'local>> {
            let text = match work(&mut *env) {
                Ok(text) => text,
                Err(error) => {
                    log_error(&format!("{name}: {error}"));
                    if env.exception_check() {
                        env.exception_clear();
                    }
                    err_json(&format!("JNI-Fehler: {error}"))
                }
            };
            JString::from_str(env, text)
        })
        .into_outcome();
    match outcome {
        Outcome::Ok(text) => text,
        Outcome::Err(error) => {
            log_error(&format!("{name}: Antwort nicht erzeugt: {error}"));
            JString::default()
        }
        Outcome::Panic(_) => {
            log_error(&format!("{name}: Panik in der JNI-Brücke"));
            JString::default()
        }
    }
}

/// The Java VM and the application context, referenced for the life of the
/// process because `ndk-context` hands out their raw pointers.
struct HostContext {
    _vm: JavaVM,
    _context: Global<JObject<'static>>,
}

static HOST_CONTEXT: OnceLock<HostContext> = OnceLock::new();
static PLATFORM_INIT: Mutex<()> = Mutex::new(());

/// `ndk-context` (once per process) and the platform TLS verifier, both
/// before the core starts (api.md §1).
fn prepare_platform(env: &mut Env<'_>, context: &JObject<'_>) -> jni::errors::Result<()> {
    let _serialized = PLATFORM_INIT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if HOST_CONTEXT.get().is_none() {
        let vm = env.get_java_vm()?;
        let global = env.new_global_ref(context)?;
        // SAFETY: the JavaVM pointer and the global context reference stay
        // valid for the whole process (both are kept in `HOST_CONTEXT`), and
        // the lock plus the `HOST_CONTEXT` check make this the only call.
        unsafe {
            ndk_context::initialize_android_context(
                vm.get_raw().cast(),
                global.as_obj().as_raw().cast(),
            );
        }
        let _ = HOST_CONTEXT.set(HostContext {
            _vm: vm,
            _context: global,
        });
    }
    let local = env.new_local_ref(context)?;
    rustls_platform_verifier::android::init_with_env(env, local)
}

/// `external fun init(context: Context, configJson: String): String`
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_init<'local>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    context: JObject<'local>,
    config_json: JString<'local>,
) -> JString<'local> {
    respond(&mut unowned, "init", |env| {
        let config = config_json.try_to_string(env)?;
        if let Err(error) = prepare_platform(env, &context) {
            log_error(&format!("init: Plattform vorbereiten: {error}"));
            if env.exception_check() {
                env.exception_clear();
            }
            return Ok(err_json(&format!(
                "Android-Anbindung fehlgeschlagen: {error}"
            )));
        }
        Ok(smart_explorer::mobile::init(&config))
    })
}

/// `external fun call(method: String, argsJson: String): String`
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_call<'local>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    method: JString<'local>,
    args_json: JString<'local>,
) -> JString<'local> {
    respond(&mut unowned, "call", |env| {
        let method = method.try_to_string(env)?;
        let args = args_json.try_to_string(env)?;
        Ok(smart_explorer::mobile::call(&method, &args))
    })
}

/// `external fun pollEvents(timeoutMs: Int): String`
#[no_mangle]
pub extern "system" fn Java_app_smartexplorer_android_core_NativeBridge_pollEvents<'local>(
    mut unowned: EnvUnowned<'local>,
    _this: JObject<'local>,
    timeout_ms: jint,
) -> JString<'local> {
    respond(&mut unowned, "pollEvents", |_env| {
        let timeout = Duration::from_millis(u64::try_from(timeout_ms).unwrap_or(0));
        Ok(smart_explorer::mobile::poll_events(timeout))
    })
}
