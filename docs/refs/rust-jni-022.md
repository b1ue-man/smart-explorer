# jni 0.22.4 / ndk-context 0.1.1 / rustls-platform-verifier 0.7.0 (Android)

Quelle: lokaler Cargo-Registry-Checkout (`/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`,
read-only via `sudo -n cat`/`sed`/`grep`) — `jni-0.22.4/src/{env.rs,errors/policy.rs,objects/jstring.rs,
refs/global.rs,vm/java_vm.rs,lib.rs,macros.rs,Cargo.toml.orig}`, `jni-0.22.4/docs/0.22-MIGRATION.md`,
`jni-0.22.4/docs/macros/{jni_str,jni_sig,jni_mangle,native_method}.md`, `ndk-context-0.1.1/src/lib.rs`,
`ndk-context-0.1.1/Cargo.toml.orig`, `rustls-platform-verifier-0.7.0/{src/android.rs,src/lib.rs,README.md,
Cargo.toml.orig}`, `rustls-platform-verifier-android-0.1.1/{Cargo.toml.orig,maven/pom.xml,
maven/rustls/rustls-platform-verifier/maven-metadata-local.xml}` · plus
[developer.android.com/ndk/reference/group/logging](https://developer.android.com/ndk/reference/group/logging) ·
Abgerufen: 2026-09-25.

All line/signature text below is copied verbatim (or lightly reformatted for width) from the actual
crate source at the exact locked versions — not from memory or docs.rs paraphrase — because this is
what will be compiled unseen by remote CI.

---

## 1. jni 0.22.4 — Cargo dependency line

From `jni-0.22.4/Cargo.toml` (registry-normalized) and `Cargo.toml.orig`:

```toml
[dependencies]
jni = "0.22.4"
```

- `default = []` — **no default features**. The only optional feature is `invocation`
  (`dep:java-locator`, `dep:libloading`), which is for **launching** a JVM from a native Rust host
  process (desktop embedding). **Not needed on Android** — Android already runs the JVM and calls
  *into* Rust; `JavaVM::new()` (the invocation-API entry point) is hard-coded to
  `Err(StartJvmError::Unsupported)` under `#[cfg(target_os = "android")]` regardless of the feature
  flag (`jni-0.22.4/src/vm/java_vm.rs:203-222`). Do **not** enable `invocation` for the Android
  `cdylib` target.
- `jni-macros = "=0.22.4"` (exact-pinned) and `jni-sys = "0.4.1"` are pulled in automatically; no
  extra Cargo lines needed for the `jni_str!`/`jni_sig!`/`jni_mangle!`/`native_method!` macros — they
  are re-exported at the crate root (`pub use jni_macros::jni_str;` etc., `lib.rs:400-423`).
- On Android specifically, `jni-0.22.4`'s own `[target.'cfg(not(target_os = "android"))'.dependencies]`
  block excludes `java-locator`/`libloading` from the Android build already, so nothing extra to gate.

## 2. The canonical native-method pattern in 0.22 — `EnvUnowned` → `with_env` → `resolve`

**This is a real, breaking rewrite vs 0.21** (see `docs/0.21-MIGRATION.md`, confirmed by reading
`env.rs`). `JNIEnv` is a deprecated alias; native methods now take `EnvUnowned<'local>` as their
first argument, not an owned `Env`/`JNIEnv`.

### 2.1 Exact shape (from `env.rs:4620-4650`, doctested `rust,no_run` example in the crate itself)

```rust
use jni::objects::{JObject, JString};
use jni::errors::ThrowRuntimeExAndDefault;

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_example_MyClass_myNativeMethod<'caller>(
    mut unowned_env: jni::EnvUnowned<'caller>,
    _this: JObject<'caller>,
    arg: JString<'caller>,
) -> JObject<'caller> {
    unowned_env.with_env(|env| -> jni::errors::Result<_> {
        // Use `env` (a `&mut jni::Env<'local>`) to call Java methods or access fields.
        Ok(JObject::null())
    }).resolve::<ThrowRuntimeExAndDefault>()
}
```

`#[unsafe(no_mangle)]` is the Rust-2024-edition spelling of the `unsafe` attribute wrapper; plain
`#[no_mangle]` still compiles (edition 2021/2024 both accept it, `#[unsafe(no_mangle)]` is only
mandatory once `unsafe_attr_outside_unsafe` is denied — **jni-0.22.4 itself requires
`rust-version = "1.85.0"`, edition `2024`**, per `Cargo.toml.orig`). Use `#[unsafe(no_mangle)]` to
match the crate's own convention and avoid an edition-2024 lint if the workspace later moves to that
edition; `smart_explorer`'s `native/Cargo.toml` is currently `edition = "2021"` where plain
`#[no_mangle]` is fine — **either form works for 0.22.4, confirm against the workspace's actual
edition before committing to one style.**

### 2.2 `EnvUnowned<'local>` (env.rs:4768-4886)

```rust
#[repr(transparent)]
pub struct EnvUnowned<'local> { /* private: raw *mut jni_sys::JNIEnv + PhantomData */ }

impl<'local> EnvUnowned<'local> {
    pub fn with_env<F, T, E>(&mut self, f: F) -> EnvOutcome<'local, T, E>
    where
        F: FnOnce(&mut Env<'local>) -> std::result::Result<T, E>,
        E: From<Error>;

    pub fn with_env_no_catch<F, T, E>(&mut self, f: F) -> EnvOutcome<'local, T, E>
    where
        F: FnOnce(&mut Env<'local>) -> std::result::Result<T, E>,
        E: From<Error>;

    pub unsafe fn from_raw(ptr: *mut jni_sys::JNIEnv) -> Self; // panics if ptr is null
    pub fn as_raw(&self) -> *mut jni_sys::JNIEnv;
    pub fn into_raw(self) -> *mut jni_sys::JNIEnv;
}
```

- **`with_env` wraps the closure in `std::panic::catch_unwind` internally** — this **is** the
  built-in unwind guard; you do **not** need to add your own `catch_unwind` around the closure body.
  Source (`env.rs:4796-4813`, doc comment on `with_env`): *"To avoid the risk of unwinding into the
  JVM (which will abort the process) this API wraps the closure in a `catch_unwind` to catch any
  panics."* — and the implementation literally does
  `catch_unwind(AssertUnwindSafe(|| f(env)))` before building the `Outcome`.
- `with_env_no_catch` is the *unguarded* variant — explicitly **not** recommended for a real native
  method entry point ("Since it would lead to undefined behaviour to allow Rust code to unwind
  across a native method call boundary, you probably want to use `EnvUnowned::with_env` instead").
- Neither variant opens a new JNI local-reference stack frame — the JVM cleans up the frame when the
  native method returns, matching normal JNI semantics.
- `EnvUnowned::from_raw` is `unsafe`; only needed if you're handed a raw `*mut jni_sys::JNIEnv`
  pointer outside of a native-method argument position (not the common case).

### 2.3 `Outcome<T, E>` / `EnvOutcome<'local, T, E>` (env.rs:4580-4770)

```rust
pub enum Outcome<T, E> { Ok(T), Err(E), Panic(Box<dyn std::any::Any + Send + 'static>) }

#[must_use = "The outcome must be resolved ... See ::resolve or ::resolve_with"]
pub struct EnvOutcome<'local, T, E> { /* raw_env ptr + outcome + PhantomData */ }

impl<'local, T, E> EnvOutcome<'local, T, E> {
    pub fn resolve<'native_method, P>(self) -> T
    where P: ErrorPolicy<T, E, Captures<'local, 'native_method> = ()>,
          T: Default + 'native_method, 'local: 'native_method;

    pub fn resolve_with<'native_method, P, F>(self, capture: F) -> T
    where P: ErrorPolicy<T, E>, T: Default + 'native_method,
          F: FnOnce() -> <P as ErrorPolicy<T, E>>::Captures<'local, 'native_method>,
          'local: 'native_method;

    pub fn into_outcome(self) -> Outcome<T, E>; // escape hatch, handle manually
}
```

`resolve`/`resolve_with` themselves also wrap the `ErrorPolicy::on_error`/`on_panic` call in a
*second* `catch_unwind`, with `on_internal_jni_error`/`on_internal_panic` as last-resort fallbacks
that log and return `T::default()` — so a policy that itself panics still can't unwind into the JVM.

### 2.4 `ErrorPolicy<T, E>` and the three built-in policies (`errors/policy.rs`, full file read)

```rust
pub trait ErrorPolicy<T, E> {
    type Captures<'unowned_env_local: 'native_method, 'native_method>;
    fn on_error<'u: 'n, 'n>(env: &mut Env<'u>, cap: &mut Self::Captures<'u, 'n>, err: E)
        -> crate::errors::Result<T>;
    fn on_panic<'u: 'n, 'n>(env: &mut Env<'u>, cap: &mut Self::Captures<'u, 'n>,
        payload: Box<dyn std::any::Any + Send + 'static>) -> crate::errors::Result<T>;
    // on_internal_jni_error / on_internal_panic have default impls (log + T::default()).
}
```

| Policy | Behavior | `Captures` |
|---|---|---|
| `ThrowRuntimeExAndDefault` | Throws `java.lang.RuntimeException` with message `"Rust error: {err}"` (or `"Rust panic: {msg}"`), unless an exception is already pending (checked via `env.exception_check()`, in which case it just returns default). Returns `T::default()` either way. | `()` (none) |
| `LogErrorAndDefault` | `log::error!("Rust error: {err}")` / `log::error!("Rust panic: {msg}")`, returns `T::default()`. No Java exception thrown. | `()` |
| `LogContextErrorAndDefault` | Same as above but formats `"{context}: {message}"`; the context string is supplied via `resolve_with::<LogContextErrorAndDefault, _>(|| format!("..."))`. | `String` |

All three require `T: Default` and `E: std::error::Error` (blanket `impl<T: Default, E:
std::error::Error> ErrorPolicy<T, E> for ...`), so your closure's `Err` type must impl
`std::error::Error` (e.g. `jni::errors::Error` itself, or your own error enum via `thiserror`).
A custom policy can capture borrowed state (e.g. a local ref argument) — see the crate's own
`CustomPolicy`/`TestCustomPolicy` example in `errors/policy.rs` doc comments / `#[cfg(test)]` module
if a captured-context policy is ever needed.

### 2.5 Full JSON-command-protocol example, 0.22.4-accurate (adapt for `smart_explorer`)

```rust
use jni::objects::{JObject, JString};
use jni::errors::ThrowRuntimeExAndDefault;

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_smartexplorer_android_NativeBridge_dispatch<'local>(
    mut unowned_env: jni::EnvUnowned<'local>,
    _class: JObject<'local>,
    request_json: JString<'local>,
) -> JString<'local> {
    unowned_env.with_env(|env| -> jni::errors::Result<JString<'local>> {
        let request: String = request_json.try_to_string(env)?;
        let command: Command = serde_json::from_str(&request)
            .map_err(|e| jni::errors::Error::JavaException /* map via your own error type instead */)?;
        let response = dispatch_command(command); // reuse existing core:: logic
        let out = serde_json::to_string(&response).expect("serde_json::to_string is infallible here");
        JString::from_str(env, out)
    }).resolve::<ThrowRuntimeExAndDefault>()
}
```

Notes:
- `serde_json::from_str` returns `serde_json::Error`, which does **not** implement
  `jni::errors::Error` directly — wrap it in a project error enum that implements
  `std::error::Error` (via `thiserror`, already a transitive-friendly pattern) and impl
  `From<Error> for YourError` (required by `with_env`'s `E: From<Error>` bound) so `?` on JNI calls
  still works inside the closure.
- Returning `JString<'local>` **directly** (not a raw `jstring`) is correct and is exactly what the
  crate's own doc examples do — see §3 below for why this is ABI-safe.

## 3. Returning `JString`/`JObject`/`jboolean`/`jlong` — no manual `.into_raw()` needed

`JObject<'local>` (and `JString`, which wraps it via the `bind_java_type!` macro) is
**`#[repr(transparent)]`** around the raw `jni_sys::jobject` pointer
(`objects/jobject.rs:58-60`: `#[repr(transparent)] pub struct JObject<'local> { ... }`), so it is
ABI-identical to the raw pointer type as an `extern "system" fn` return value — the crate's own
doctested examples (which are `rust,no_run` — i.e. type-checked by `cargo test --doc`) return
`-> JObject<'local>` / `-> JString<'local>` directly from `Java_...` functions, not `-> jstring`.
Both `JObject` and `JString` implement `Default` (`objects/jobject.rs:164`, a null-wrapping impl),
which satisfies `EnvOutcome::resolve`'s `T: Default` bound (the "default" returned on the error path
is a Java `null`).

For primitive returns (`jboolean`, `jlong`, `jint`, ...), the raw `jni::sys` type aliases (`type
jboolean = u8; type jlong = i64; ...`, re-exported via `jni::sys::*`, `jni_sys` crate) already
implement `Default` (0 / `JNI_FALSE`), so e.g.:

```rust
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_example_MyClass_isReady<'local>(
    mut unowned_env: jni::EnvUnowned<'local>,
    _this: JObject<'local>,
) -> jni::sys::jboolean {
    unowned_env.with_env(|_env| -> jni::errors::Result<jni::sys::jboolean> {
        Ok(1u8) // JNI_TRUE
    }).resolve::<jni::errors::LogErrorAndDefault>() // returns 0 (JNI_FALSE) on error
}
```

works with no extra conversion. If you need the raw `jstring`/`jobject` type explicitly (e.g. to
match an existing FFI boundary), `Reference::as_raw(&self) -> jobject` and consuming
`JObject::into_raw(self) -> jobject` (inherent, mirrors `Reference`) are still available — but are
not required for a plain native-method return.

## 4. `JString` ↔ `String` conversions (0.22.4 exact method names) — `objects/jstring.rs`, full file read

| Direction | Method (0.22.4) | Signature | Notes |
|---|---|---|---|
| `&str`/`String` → `JString` | `JString::from_str` | `fn from_str<'e>(env: &mut Env<'e>, from: impl AsRef<str>) -> Result<JString<'e>>` | Re-encodes to MUTF-8, allocates. **Preferred general-purpose constructor.** |
| same, alias | `JString::new` | `fn new<'e>(env: &mut Env<'e>, from: impl AsRef<str>) -> Result<JString<'e>>` | "convenience that's equivalent to calling `Self::from_str`". |
| compile-time MUTF-8 literal → `JString` | `JString::from_jni_str` | `fn from_jni_str<'e>(env: &mut Env<'e>, from: impl AsRef<JNIStr>) -> Result<JString<'e>>` | Pair with the `jni_str!` macro (§6) to avoid a runtime re-encode for literals/constant strings. |
| `JString` → MUTF-8 byte guard | `JString::mutf8_chars` | `fn mutf8_chars(&self, env: &Env<'_>) -> Result<MUTF8Chars<'_, &JString<'_>>>` | **The 0.22 replacement for 0.21's `env.get_string()`.** Guard derefs to `JNIStr`; releases the native chars on `Drop`. Errors: `Error::NullPtr` if the `JString` is null. |
| `JString` → owned `String` | `JString::try_to_string` | `fn try_to_string(&self, env: &Env<'_>) -> Result<String>` | Equivalent to `mutf8_chars(env)?.to_string()`. **This is the direct 0.22 replacement for the old "get_string then `.into()`" 0.21 idiom** — use this for the common case. Errors: `Error::NullPtr` if null. |
| `JString`/`MUTF8Chars` → `String` via `Display`/`ToString` | `.to_string()` | inherent via `impl Display for JString` | Also works, but silently prints `"<NULL>"` / `"<JNI Error>"` / `"<JNI Not Initialized>"` instead of returning a `Result` on failure — **prefer `try_to_string` in a native-method body** so errors surface through your `ErrorPolicy` instead of being swallowed into a placeholder string. |

Task-brief method names cross-check: `mutf8_chars` ✅ exists (0.22.4); a bare `get_string` method
does **not** exist on `JString`/`Env` in 0.22.4 (the 0.21 `Env::get_string(&jstr)` method was
removed, not merely deprecated, per `docs/0.21-MIGRATION.md`); `try_to_string` ✅ exists and is the
recommended one-shot conversion.

## 5. `JObject` (Android `Context`) → global ref; `JavaVM` from `Env`; raw pointers for `ndk-context`

### 5.1 `Env::new_global_ref` (env.rs:1379-1420, full excerpt read)

```rust
pub fn new_global_ref<'any_local, O>(&self, obj: O) -> Result<Global<O::GlobalKind>>
where O: Reference + AsRef<JObject<'any_local>>;
```

- `GlobalRef` was **renamed to `Global<T>`** in 0.22.0 (`refs/global.rs:104-112`); `pub type
  GlobalRef<T> = Global<T>` still exists but is `#[deprecated(since = "0.22.0", note = "... has been
  renamed to `Global`")]` — **use `Global<T>` in new code**, e.g. `Global<JObject<'static>>` for a
  stored Android `Context`.
- `Global<T>` is `Send + Sync`, not tied to any `Env`/thread lifetime; `Global::as_obj(&self) ->
  &JObject<'static>` borrows it back; `Global::into_raw(self) -> sys::jobject` consumes it (you then
  own cleanup); dropping a `Global` off an attached thread logs a `log::Level::Warn` performance
  warning and pays a temporary-attach cost (`refs/global.rs:280-310`) — **drop global refs from an
  already-attached thread where possible.**

### 5.2 `Env::get_java_vm` (env.rs:4433) and `JavaVM` (vm/java_vm.rs, full relevant excerpts read)

```rust
// On Env<'_>:
pub fn get_java_vm(&self) -> Result<JavaVM>; // may return Error::JavaException if called with a pending exception

// On JavaVM:
pub fn singleton() -> Result<Self>;                       // global process-wide singleton, once observed
pub unsafe fn from_raw(ptr: *mut sys::JavaVM) -> Self;     // also seeds the singleton (JAVA_VM_SINGLETON.get_or_init)
pub fn get_raw(&self) -> *mut sys::JavaVM;                 // raw pointer back out
```

- `JavaVM::singleton()` reflects a genuine `OnceLock`-style global — once any code path has called
  `JavaVM::from_raw` (or a native method has run through `EnvUnowned::with_env` once, which
  internally establishes it), `JavaVM::singleton()` succeeds from anywhere in the process,
  independent of which specific `jni-rs` call path first saw the pointer. **Caveat explicitly called
  out in the doc comment:** a *different copy* of the `jni-rs` crate (e.g. pulled in transitively at
  a different semver-incompatible version by another dependency) does **not** share this singleton —
  only matters if two different `jni = "0.2x"` majors end up in the same dependency graph.

### 5.3 Raw pointers for `ndk-context` (see §7) — how to get `*mut c_void` from `jni` types

```rust
// From a `JavaVM` (e.g. env.get_java_vm()?):
let java_vm_raw: *mut jni::sys::JavaVM = java_vm.get_raw();
// cast to *mut c_void for ndk_context::initialize_android_context's first argument.

// From a `Global<JObject<'static>>` (e.g. env.new_global_ref(context_jobject)?):
let context_raw: jni::sys::jobject = global_context.as_obj().as_raw(); // via Reference::as_raw
// jni::sys::jobject is itself a raw pointer typedef; cast to *mut c_void for the second argument.
```
(`Reference::as_raw(&self) -> jobject` is the trait method backing this — confirmed in
`refs/reference.rs:231`, and re-implemented for `Global<T>` at `refs/global.rs:~300` to forward to
the wrapped `T::as_raw`.)

## 6. Useful 0.22-only macros (re-exported at crate root, `lib.rs:400-423`; full macro docs read)

| Macro | Purpose | Example |
|---|---|---|
| `jni_str!("literal")` | Compile-time UTF-8→MUTF-8 encode to a `&'static JNIStr`; supports concatenation (`jni_str!("a", "b")`) and mixed literal types. Pair with `JString::from_jni_str` to skip a runtime encode. | `const CLASS: &JNIStr = jni_str!("java.lang.String");` |
| `jni_sig!(...)` | Compile-time method/field signature parser: `jni_sig!((arg: JString, n: jint) -> JString)` → a `MethodSignature` with the raw JNI sig string (`"(Ljava/lang/String;I)Ljava/lang/String;"`) precomputed. Also accepts raw `"(...)..."` strings for validation. | `const SIG: MethodSignature = jni_sig!((a: jint) -> void);` |
| `jni_mangle!(...)` (attribute, `#[jni_mangle("com.pkg.Class")]`) | Generates the correctly-mangled `Java_com_pkg_Class_methodName` `#[export_name]` **and** forces `extern "system"` ABI, from a plain Rust fn — `snake_case` Rust fn names are auto-converted to `lowerCamelCase` Java method names (unless the name already contains uppercase, in which case it's used verbatim). Accepts 1-3 string-literal args: namespace, optional method-name-or-signature, optional signature (for overload disambiguation). | `#[jni_mangle("com.smartexplorer.android.NativeBridge")] pub fn dispatch<'l>(env: EnvUnowned<'l>, _t: JObject<'l>, req: JString<'l>) -> JString<'l> { .. }` → exports `Java_com_smartexplorer_android_NativeBridge_dispatch`. Saves hand-writing/hand-mangling the `Java_...` name, especially useful if the JVM package/class name has underscores (which the raw JNI mangling scheme also escapes specially) — `jni_mangle!` handles that correctly where a hand-written name is error-prone. |
| `native_method!` | Higher-level macro to declare `NativeMethod` descriptors for `Env::register_native_methods` (explicit registration path, alternative to relying on the JVM's automatic `Java_...` symbol lookup) — not required unless dynamic/explicit method registration is wanted instead of static `Java_...` exports. | (not needed for a static `Java_...`-export bridge; skip unless explicit registration is chosen) |

## 7. `ndk-context` 0.1.1 — `initialize_android_context` / `android_context` / `release_android_context`

Full source read (86 lines, `ndk-context-0.1.1/src/lib.rs`):

```rust
use std::ffi::c_void;

#[derive(Clone, Copy, Debug)]
pub struct AndroidContext { /* private: java_vm: *mut c_void, context_jobject: *mut c_void */ }

impl AndroidContext {
    pub fn vm(self) -> *mut c_void;       // the JavaVM* handle
    pub fn context(self) -> *mut c_void;  // the android.content.Context jobject handle
}

pub fn android_context() -> AndroidContext;
// panics with message "android context was not initialized" (an `.expect(...)` on the static Option)
// if called before `initialize_android_context`.

/// # Safety
/// The pointers must be valid and this function must be called exactly once before `main` is called.
pub unsafe fn initialize_android_context(java_vm: *mut c_void, context_jobject: *mut c_void);
// internally: `ANDROID_CONTEXT.replace(...)` then `assert!(previous.is_none())` —
// **panics if called a second time** (the `assert!` fires on any re-init attempt).

/// # Safety
/// Must only be called after `initialize_android_context()`, when the activity is destroyed.
pub unsafe fn release_android_context();
// internally: `ANDROID_CONTEXT.take()` then `assert!(previous.is_some())` —
// **panics if called without a prior successful init** (asymmetric misuse also panics).
```

**Cargo line:** `ndk-context = "0.1.1"` (no features; `Cargo.toml.orig` declares none).

**Pitfalls (from reading the source directly, not paraphrased):**
- The crate's own doc says it's "usually" initialized by `ndk-glue` "before `main` is called" — but
  in a JNI `cdylib` loaded by a normal Android/Kotlin `Activity` (not an `ndk-glue`/`android-activity`
  `NativeActivity`), **nothing calls this automatically**. The app's own JNI init path (an explicit
  `external fun init(context: Context)` call from Kotlin at `Activity.onCreate()`, or a `JNI_OnLoad`
  if one is implemented) must call `ndk_context::initialize_android_context(vm_raw, context_raw)`
  itself, exactly once, before any code that transitively reaches it (see `docs/refs/android-rust-deps.md`
  §2 for the confirmed real crash — `hickory-resolver`'s Android DNS backend calls
  `ndk_context::android_context()` and **panics** with `"android context was not initialized"` if this
  step was skipped or raced).
- Both `initialize_android_context` and `release_android_context` are `unsafe fn` with a documented
  single-call invariant enforced by an internal `assert!` — calling `initialize_android_context`
  **twice** (e.g. once from a `JNI_OnLoad` and again from an explicit Kotlin-triggered init call) will
  **panic** the process. Guard with a `std::sync::Once`/`OnceLock` in the app's own init wrapper if
  there's any chance of being called from more than one path (e.g. both a cold `JNI_OnLoad` and a
  warm re-entry after `Activity` recreation).
- `ANDROID_CONTEXT` is a bare `static mut` (not an atomic/`OnceLock`) guarded only by the caller's
  promise to call init "before `main`"/exactly once — there is **no internal synchronization**; if
  the app calls this from a background thread concurrently with the first `android_context()` read,
  that is a data race the crate does not protect against. In a JNI-loaded (not `ndk-glue`) app, do
  the init call synchronously on the same thread/call path that will first use it (e.g. directly in
  the `Java_..._init` native method), not from a spawned worker thread.

## 8. `rustls-platform-verifier` 0.7.0 on Android

### 8.1 Cargo (`Cargo.toml.orig`, Android-specific target section, exact)

```toml
[target.'cfg(target_os = "android")'.dependencies]
once_cell = "1.9"
rustls-platform-verifier-android = { path = "../android-release-support", version = "0.1.0" }
jni = { version = "0.22", default-features = false }
webpki = { package = "rustls-webpki", version = "0.103", default-features = false }
android_logger = { version = "0.15", optional = true } # only for the crate's own `ffi-testing` feature
```
This is automatically pulled in by depending on plain `rustls-platform-verifier = "0.7.0"` — no
manual Android-specific dependency lines are needed in `smart_explorer`'s own `Cargo.toml` beyond
what's already there per `docs/refs/android-rust-deps.md` §1/§3 (already a transitive dependency via
`iroh` → `reqwest`). The `version = "0.1.0"` bound on `rustls-platform-verifier-android` is
satisfied by the actually-locked `0.1.1` (semver-compatible).

### 8.2 Required init call — **actual 0.7.0 source function names** (`src/android.rs`, full file read)

```rust
/// Initialize given a typical Android NDK `Env` and `JObject` context.
pub fn init_with_env(env: &mut jni::Env, context: jni::objects::JObject) -> Result<(), jni::errors::Error>;

/// Initialize with a `&'static dyn Runtime` that dynamically serves JVM/context/class-loader refs.
pub fn init_with_runtime(runtime: &'static dyn Runtime); // never panics

/// Initialize with pre-obtained JavaVM + global refs (context, class loader) directly.
pub fn init_with_refs(
    java_vm: jni::JavaVM,
    context: jni::objects::Global<jni::objects::JObject<'static>>,
    loader: jni::objects::Global<jni::objects::JClassLoader<'static>>,
); // never panics
```

**⚠️ Contradiction, flagged explicitly:** the crate's checked-out `README.md` (same 0.7.0 tarball)
documents **different, older function names** — `rustls_platform_verifier::android::init_hosted(&env,
context)` and `init_external(...)`, with an old-style `env: JNIEnv` (0.21-shaped) signature in its
code sample. Those names/signatures **do not exist** in the actual `src/android.rs` of this exact
0.7.0 checkout — only `init_with_env` / `init_with_runtime` / `init_with_refs` are defined there,
matching jni **0.22**'s `Env`/`Global`/`EnvUnowned` types. **Treat `src/android.rs` as ground truth**
(it's what actually compiles) and the README's `init_hosted`/`init_external` names as stale
documentation that was not updated for this release — do not call `init_hosted`/`init_external`,
they will fail to resolve.

Minimal, 0.22-API-correct init call from a JNI entry point (adapting the pattern the crate's own
`src/android.rs` doc-comment sketches, corrected to match the real `with_env` signature):

```rust
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_smartexplorer_android_NativeBridge_initTls<'local>(
    mut unowned_env: jni::EnvUnowned<'local>,
    _class: jni::objects::JObject<'local>,
    context: jni::objects::JObject<'local>,
) -> jni::sys::jboolean {
    unowned_env.with_env(|env| -> jni::errors::Result<jni::sys::jboolean> {
        rustls_platform_verifier::android::init_with_env(env, context)
            .map_err(|_e| jni::errors::Error::JniCall(jni::errors::JniError::Unknown))?;
        Ok(1u8)
    }).resolve::<jni::errors::LogErrorAndDefault>()
}
```
(`init_with_env`'s own error type is `jni::errors::Error`, so mapping it 1:1 back into the closure's
`Result<_, jni::errors::Error>` and skipping the extra `map_err` is simpler in practice — shown here
only to illustrate error-type plumbing if a project-local error enum is used instead.)

### 8.3 What happens if not initialized — exact panic message

From `src/android.rs`'s private `global()` accessor (used by every verification call path):
```rust
fn global() -> &'static GlobalStorage {
    GLOBAL.get().expect("Expect rustls-platform-verifier to be initialized")
}
```
→ **panic message is exactly `"Expect rustls-platform-verifier to be initialized"`**, raised on the
*first TLS verification attempt* (not at crate load), if none of `init_with_env` /
`init_with_runtime` / `init_with_refs` was called first. Since this happens inside `rustls`'s
certificate-verifier callback (itself likely called from Tokio/network code, not from a JNI-called
stack frame with an `EnvUnowned` guard around it), **this panic will not be caught by any
`EnvUnowned::with_env` `catch_unwind`** — it will unwind through ordinary Rust code and, depending on
where the network stack runs it (a Tokio worker thread vs. the JNI-calling thread), may abort the
process. **Call one of the three init functions unconditionally during app/library startup, before
any `ClientConfig::builder_with_provider(...).with_platform_verifier()` path can run.**

### 8.4 Kotlin/Gradle side (from `README.md`, still valid — only the Rust-side function names in §8.2
are stale, the Gradle/Maven wiring is unaffected)

**Maven repository pointing at the bundled Android artifact**, located via `cargo metadata`:
```groovy
// build.gradle (Groovy)
repositories {
    maven {
        url = findRustlsPlatformVerifierProject()
        metadataSources.artifact()
    }
}
String findRustlsPlatformVerifierProject() {
    def dependencyText = providers.exec {
        it.workingDir = new File("../")
        commandLine("cargo", "metadata", "--format-version", "1",
                    "--filter-platform", "aarch64-linux-android",
                    "--manifest-path", "$PATH_TO_DEPENDENT_CRATE/Cargo.toml")
    }.standardOutput.asText.get()
    def dependencyJson = new JsonSlurper().parseText(dependencyText)
    def manifestPath = file(dependencyJson.packages.find {
        it.name == "rustls-platform-verifier-android"
    }.manifest_path)
    return new File(manifestPath.parentFile, "maven").path
}
dependencies {
    implementation "rustls:rustls-platform-verifier:latest.release"
}
```
Kotlin-DSL (`.gradle.kts`) equivalent is in the README with the same `cargo metadata --filter-platform
aarch64-linux-android` invocation — the `--filter-platform` argument matters: without it, `cargo
metadata` may not include the Android-target-only dependency at all in some workspace configurations.

**Confirmed dependency coordinate & version (read directly from the vendored Maven repo, not
inferred):**
- `rustls-platform-verifier-android-0.1.1/maven/pom.xml`: `<groupId>rustls</groupId>
  <artifactId>rustls-platform-verifier</artifactId> <version>0.1.1</version> <packaging>aar</packaging>`
- `.../maven/rustls/rustls-platform-verifier/maven-metadata-local.xml`: `<release>0.1.1</release>`
- **The Maven artifact version (`0.1.1`) is the `rustls-platform-verifier-android` crate's own
  version, not the parent `rustls-platform-verifier` crate's version (`0.7.0`).** Do not assume they
  move in lockstep — always resolve the Gradle coordinate via the `cargo metadata` script above (or
  `latest.release` as the README's own snippets do) rather than hand-pinning `0.7.0`.
- The `.aar` payload itself is physically present at
  `.../maven/rustls/rustls-platform-verifier/0.1.1/rustls-platform-verifier-0.1.1.aar` in the vendored
  crate — confirms the "bundled in the crate" claim; a CI script can `find` this path directly as a
  sanity check that `cargo metadata`'s `manifest_path`-derived `maven/` dir resolution worked.

**CI-robust way to locate the maven dir** (per task brief, using `jq` instead of Groovy's
`JsonSlurper` — same underlying `cargo metadata` call):
```bash
MANIFEST_PATH=$(cargo metadata --format-version 1 \
    --filter-platform aarch64-linux-android \
    --manifest-path native/Cargo.toml \
  | jq -r '.packages[] | select(.name == "rustls-platform-verifier-android") | .manifest_path')
MAVEN_DIR="$(dirname "$MANIFEST_PATH")/maven"
```

**Ergänzung K0 (2026-09-25).** Im Versionsordner des gebündelten Repos liegen
`_remote.repositories`, `rustls-platform-verifier-0.1.1.aar` und `rustls-platform-verifier-0.1.1.pom`
(`<groupId>rustls</groupId>`, `<packaging>aar</packaging>`; docs.rs-Quellansicht der Crate 0.1.1) –
Standard-Metadatenquellen (POM) genügen, `metadataSources.artifact()` ist nicht nötig. Die aktuelle
Upstream-README verweist inzwischen auf ein gehostetes Maven-Archiv und liest die Version per
`ValueSource` aus `Cargo.lock` (Paket `rustls-platform-verifier-android`). Dieses Projekt: Gradle-
Property `rustlsVerifierMaven` (Pfad zum `maven/`-Ordner der Crate, per `cargo metadata
--filter-platform aarch64-linux-android` ermittelt) als `exclusiveContent`-Repository nur für die
Gruppe `rustls` in `android/settings.gradle.kts`; die Version liest `android/app/build.gradle.kts`
aus `native/Cargo.lock`.

**Proguard/R8 keep rule** (README, exact text):
```text
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }
```
(Needed because Proguard/R8 cannot see JNI-only usage and would otherwise strip the Kotlin component
as apparently-dead code.)

## 9. `android_logger` alternative — raw `__android_log_write` NDK C API

(For cases where pulling in the `android_logger`/`log` crate pair — already covered in
`docs/refs/rust-jni.md` §3 — is undesired and a direct NDK liblog call is preferred instead.)

Exact C signature, from
[developer.android.com/ndk/reference/group/logging](https://developer.android.com/ndk/reference/group/logging)
(`<android/log.h>`), checked 2026-09-25:
```c
int __android_log_write(int prio, const char *tag, const char *text);
// Returns 1 if the message was written to the log, -EPERM if it was not.
```
```c
enum android_LogPriority {
    ANDROID_LOG_UNKNOWN = 0,
    ANDROID_LOG_DEFAULT,   // 1
    ANDROID_LOG_VERBOSE,   // 2
    ANDROID_LOG_DEBUG,     // 3
    ANDROID_LOG_INFO,      // 4
    ANDROID_LOG_WARN,      // 5
    ANDROID_LOG_ERROR,     // 6
    ANDROID_LOG_FATAL,     // 7
    ANDROID_LOG_SILENT,    // 8
};
```
Rust FFI binding pattern (standard `extern "C"` + `#[link(name = "log")]` to link `liblog.so`, which
is always present on-device — part of Bionic/the NDK's stable C API surface):
```rust
use std::ffi::{c_char, c_int, CString};

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

pub fn log_info(tag: &str, msg: &str) {
    let Ok(tag_c) = CString::new(tag) else { return };
    let Ok(msg_c) = CString::new(msg) else { return }; // CString::new fails on embedded NUL bytes
    unsafe { __android_log_write(4 /* ANDROID_LOG_INFO */, tag_c.as_ptr(), msg_c.as_ptr()); }
}
```
Pitfalls:
- `CString::new` fails (`Err(NulError)`) if `msg`/`tag` contain an embedded `\0` — must be handled
  (truncate, escape, or early-return) rather than `.unwrap()`ed in a production logging path.
  `unsafe extern "C"` block syntax matches Rust 2024's `unsafe extern` requirement; `extern "C" {
  ... }` (no `unsafe` keyword before `extern`) is the pre-2024-edition form and is what the current
  `edition = "2021"` `native/Cargo.toml` should use instead (`extern "C" { fn __android_log_write(...)
  -> c_int; }`, functions marked `unsafe fn` individually or the whole block treated as unsafe-to-call
  per pre-2024 rules).
- `#[link(name = "log")]` links `liblog.so`; this is resolved by the Android dynamic linker at
  runtime from the device's system image — `cargo-ndk`/the NDK toolchain does **not** need to bundle
  `liblog.so` into the APK, it's always present. No `jniLibs` packaging changes needed for this
  specific dependency.
- This raw API has **no log-tag length or message-length documented hard limit** in the reference
  page fetched; historical Android logcat buffer behavior truncates very long single messages
  (commonly cited practical ceiling ~4000 bytes per line for the older `liblog` ring-buffer transport)
  — not independently re-verified against a primary source in this pass; if very long single log
  lines are needed, chunk them rather than relying on an unverified limit.

---

## Unresolved / contradictions carried forward

- **README vs. source mismatch in `rustls-platform-verifier` 0.7.0** (§8.2): the README's
  `init_hosted`/`init_external` names and `JNIEnv`-shaped example are stale; the actual compiled
  `src/android.rs` API is `init_with_env`/`init_with_runtime`/`init_with_refs`. Flagged and resolved
  in favor of the source in this document — do not follow the README's Rust code sample verbatim.
- `jni-macros-0.22.4`'s own source directory was not separately enumerated file-by-file (only its
  effects, observed via `jni-0.22.4/lib.rs`'s re-exports and `jni-0.22.4/docs/macros/*.md`, which are
  authoritative for macro *behavior* since they're the crate's own compiled doc source) — if a
  `jni_sig!`/`bind_java_type!` edge case not covered by the quoted docs comes up during
  implementation, re-read `jni-macros-0.22.4/src/` directly rather than guessing.
- The exact byte/length ceiling for a single `__android_log_write` call (§9) was not confirmed
  against a primary source in this pass (developer.android.com's logging reference page did not state
  one); treat the commonly-cited ~4000-byte figure as folklore, not a verified limit.

## Ergänzung B2 (2026-09-25, Quelle: Registry-Checkout `jni-0.22.4/src`, `jni-macros-0.22.4/src`)

Für die Brücke zusätzlich gegen die Quelle geprüft (nicht aus dem Gedächtnis):
- `jni::Outcome<T, E>` (`env.rs:4586`, `pub use env::*` in `lib.rs:346`) mit `Ok(T)`, `Err(E)`,
  `Panic(Box<dyn Any + Send>)`; `EnvOutcome::into_outcome(self) -> Outcome<T, E>` (`env.rs:4738`).
  Damit lässt sich ein Fehlerpfad selbst behandeln (loggen, `JString::default()`), statt eine
  `ErrorPolicy` zu wählen, die eine Java-Exception wirft.
- `JString` wird von `bind_java_type!` mit `#[derive(Debug, Default)]` erzeugt
  (`jni-macros-0.22.4/src/bind_java_type.rs:1566`); `Default` = Java-`null`.
- `Env::new_local_ref<'any, O>(&mut self, obj: O) -> Result<O::Kind<'local>>` mit
  `O: Reference + AsRef<JObject<'any>>` (`env.rs:1738`); `Reference` ist auch für `&T`
  implementiert (`refs/reference.rs:431`), daher gehen `env.new_local_ref(&context)` und
  `env.new_global_ref(&context)` (liefert `Global<JObject<'static>>`).
- `Env::exception_check(&self) -> bool` (`env.rs:1139`), `Env::exception_clear(&self)` (`env.rs:1171`).
- `JavaVM` und `Global<T>` sind `Send + Sync` (`vm/java_vm.rs:189-190`, `refs/global.rs:112-123`),
  also in einem `static OnceLock` haltbar.
- `rustls_platform_verifier::android::init_with_env(env: &mut Env, context: JObject)` ist über
  `GLOBAL.get_or_try_init` idempotent (`rustls-platform-verifier-0.7.0/src/android.rs:97-116`); ein
  zweiter Aufruf ist harmlos. `ndk_context::initialize_android_context` dagegen nur einmal je Prozess.
