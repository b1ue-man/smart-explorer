# Rust ↔ Kotlin binding approach reference (Sept 2026)

**Purpose.** Web-research comparison of the two mainstream ways to call the existing Rust core
(`native/`, crate `smart_explorer`, Rust edition 2021 — `native/Cargo.toml:4`) from Kotlin on
Android: **UniFFI** (codegen) vs. the **`jni` crate** (hand-written JNI, matching a JSON-string
command-protocol style already implied by the assignment). Facts only, no recommendation on which
to pick. Checked 2026-09-25.

**Files read.**
- `native/Cargo.toml` (`edition = "2021"`, package `smart_explorer` — `native/Cargo.toml:1-4`; the
  crate currently has no Android-specific target section, only `cfg(windows)` /
  `cfg(not(windows))` / `cfg(target_os = "linux")` splits — `native/Cargo.toml:108,164,168`)
- Web sources cited inline below (docs.rs, crates.io, github.com/mozilla/uniffi-rs,
  github.com/jni-rs/jni-rs, developer.android.com), checked 2026-09-25.

---

## 1. Option A — UniFFI

| Item | Value (checked 2026-09-25) |
|---|---|
| Latest version | **0.32.2**, published **2026-09-23** (crates.io `newest_version`, `crates.io/api/v1/crates/uniffi`) |
| Maturity | Explicitly **pre-1.0**: "ready for production use, but a long way from a 1.0 release", breaking changes possible for "advanced" usage across upgrades. [UniFFI user guide](https://mozilla.github.io/uniffi-rs/) |
| Interface definition | Two modes: (1) legacy **UDL file** (`.udl`, a separate IDL) mode; (2) **proc-macro mode** — annotate Rust directly with `#[uniffi::export]` on functions / `impl` blocks / traits, no UDL file needed. [Proc-macro docs](https://mozilla.github.io/uniffi-rs/0.27/proc_macro/index.html) |
| Kotlin binding generation — "library mode" | Build the compiled `cdylib`/`.so` first, then run `uniffi-bindgen` against the **built library** (not the source) so it can introspect exported symbols across possibly multiple UniFFI-using crates in one workspace: `cargo build --release` then `cargo run --bin uniffi-bindgen generate --library target/release/libarithmetical.so --language kotlin --out-dir out`. Library mode requires running from within the cargo workspace and (per UDL-mode legacy constraint noted in docs) each crate uses exactly one UDL file if UDL mode is mixed in. [Foreign-language bindings guide](https://mozilla.github.io/uniffi-rs/0.27/tutorial/foreign_language_bindings.html) |
| Gradle-side dependency cost | UniFFI's Kotlin/Android bindings call into the native library **through JNA**, not raw JNI: adds `net.java.dev.jna:jna` (5.12.0 minimum per the UniFFI Gradle doc; a separate source names 5.19.1 as what recent plugin versions add) as an `@aar` dependency to `androidMain`/`jvmMain`. [Integrating with Gradle](https://mozilla.github.io/uniffi-rs/latest/kotlin/gradle.html) |
| JNA runtime cost | JNA must **attach/detach a Java thread into native on every call** ("a heavy operation" per source), plus **one extra FFI call per exported function at startup** for a signature/version check. This is the concrete "JNA cost" the assignment asked to quantify — it is a per-call and startup overhead layered *on top of* the JNI transition itself, not present when calling raw JNI directly. |
| Async support | Needs `kotlinx-coroutines-core` ≥ 1.6 on the Kotlin side for `Future`/async exported functions. |
| Example Gradle wiring | ```kotlin
dependencies {
    implementation("net.java.dev.jna:jna:5.12.0@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.6.4")
}
android.libraryVariants.all { variant ->
    val generateBindings = tasks.register<Exec>("generate${variant.name.replaceFirstChar{it.uppercase()}}UniFFIBindings") {
        workingDir(project.projectDir)
        commandLine("uniffi-bindgen", "generate", "<PATH_TO_UDL_OR_LIB>",
            "--language", "kotlin", "--out-dir", "${'$'}{buildDir}/generated/source/uniffi/${'$'}{variant.name}/java")
    }
    variant.javaCompileProvider.get().dependsOn(generateBindings)
}
``` |

## 2. Option B — the `jni` crate (hand-written JNI)

### 2.1 Version landscape — a real breaking split

| Item | Value |
|---|---|
| Latest release | **0.22.4**, published **2026-03-16** (crates.io `newest_version`). [jni crate](https://crates.io/crates/jni), [docs.rs/jni](https://docs.rs/jni) |
| **0.21.x → 0.22.x is a non-additive rewrite.** Per the jni-rs CHANGELOG: *"0.21.x APIs are not supported in 0.22.x."* Projects that want the classic `JNIEnv`-by-value API (matching the signature shape the assignment describes: `fn Java_<pkg>_<Class>_<method>(mut env: JNIEnv, _class: JClass, arg: JString) -> jstring`) must **pin `jni = "0.21"`**, not `"0.22"` or a caret range spanning both. [jni-rs CHANGELOG](https://github.com/jni-rs/jni-rs/blob/master/CHANGELOG.md) |

Concrete differences (0.21 vs 0.22+):

| Aspect | jni 0.21.x (classic, matches assignment's requested signature) | jni 0.22.x (current latest, breaking redesign) |
|---|---|---|
| Env type | Single `JNIEnv<'local>`, passed **by value** into the native fn, used directly for all calls | `JNIEnv` becomes a **deprecated alias for `EnvUnowned`**; a new `Env` type is the real interface. `EnvUnowned` (received in the native fn signature) must be upgraded via `unowned_env.with_env(|env| { ... })` before any JNI call is legal; `Env` can only be **borrowed** from an `AttachGuard`, never held by value. |
| Native fn shape | `pub extern "system" fn Java_pkg_Class_method(mut env: JNIEnv, _class: JClass, arg: JString) -> jstring` | `pub extern "system" fn Java_pkg_Class_method<'caller>(mut unowned_env: EnvUnowned<'caller>, _class: JClass<'caller>, arg: JString<'caller>) -> JString<'caller>` with body wrapped in `unowned_env.with_env(|env| { ... })` and `.resolve::<ThrowRuntimeExAndDefault>()` for error/exception mapping |
| `get_string` | `env.get_string(&jstr)? ` → `JavaStr`, `.into()` to `String` (still valid in 0.21) | `get_string` **deprecated** in favor of `JString::mutf8_chars()` |
| `new_string` | `env.new_string("foo")?` → `JString` | Conceptually replaced by `JString::from_str(env, rust_string)` in the new API shape |
| Thread attach for callbacks | `let env = vm.attach_current_thread()?;` (simple `AttachGuard` deref to `JNIEnv`) | Attach APIs redesigned around stack-pinned `AttachGuard`s and `FnOnce` closures (`attach_current_thread`, `attach_current_thread_for_scope`); `JavaVM::get_env` replaced by `JavaVM::get_env_attachment` |
| Exceptions | `env.throw_new("java/lang/RuntimeException", msg)?` then return; caller must avoid further unsafe JNI calls until cleared | `Env::throw*` now returns `Error::JavaException`, and an `AttachmentExceptionPolicy` governs stash/re-throw behavior across attach boundaries |
| Verdict for this project | **Use 0.21.x if matching common tutorials/existing StackOverflow-era code and the simplest mental model is preferred**; 0.22.x is safer (fixes real unsound edge cases around `Env` lifetimes) but is a materially different, less-documented-in-the-wild API as of the check date. | |

### 2.2 Exact current API surface used for a JSON-string command protocol (0.21.x signatures, still resolvable on docs.rs)

```rust
// String in/out
pub fn get_string<'other_local: 'obj_ref, 'obj_ref>(
    &mut self,
    obj: &'obj_ref JString<'other_local>,
) -> Result<JavaStr<'local, 'other_local, 'obj_ref>>;              // JNIEnv::get_string

pub fn new_string<S: Into<JNIString>>(&self, from: S) -> Result<JString<'local>>; // JNIEnv::new_string

// Byte arrays (for a binary payload alternative to JSON strings)
pub fn new_byte_array(&self, length: jsize) -> Result<JByteArray<'local>>;
pub fn convert_byte_array<'o>(&self, array: impl AsRef<JByteArray<'o>>) -> Result<Vec<u8>>;
pub fn set_byte_array_region<'o>(&self, array: impl AsRef<JByteArray<'o>>, start: jsize, buf: &[jbyte]) -> Result<()>;

// Exceptions
pub fn throw_new<'o, S: Into<JNIString>, T: Desc<'local, JClass<'o>>>(&mut self, class: T, msg: S) -> Result<()>;
pub fn throw<'o, E: Desc<'local, JThrowable<'o>>>(&mut self, obj: E) -> Result<()>;
pub fn exception_check(&self) -> Result<bool>;
```
Source: [docs.rs jni 0.21.1 JNIEnv](https://docs.rs/jni/0.21.1/jni/struct.JNIEnv.html).

A single-function **JSON-string command protocol** (one or two JNI entry points instead of one
exported function per Rust API call) looks like:
```rust
#[no_mangle]
pub extern "system" fn Java_com_smartexplorer_android_NativeBridge_dispatch<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    request_json: JString<'local>,
) -> jstring {
    let result: Result<String, String> = (|| {
        let request: String = env
            .get_string(&request_json)
            .map_err(|e| e.to_string())?
            .into();
        let command: Command = serde_json::from_str(&request).map_err(|e| e.to_string())?;
        let response = dispatch_command(command); // reuse existing core:: logic
        serde_json::to_string(&response).map_err(|e| e.to_string())
    })();

    let out = match result {
        Ok(json) => json,
        Err(err) => {
            let _ = env.throw_new("java/lang/RuntimeException", &err);
            return std::ptr::null_mut();
        }
    };
    env.new_string(out)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}
```
This reuses `serde` / `serde_json`, both **already dependencies of `smart_explorer`**
(`native/Cargo.toml:60-61`), so a JSON command/response protocol adds no new serialization
dependency versus what the desktop crate already carries.

### 2.3 Kotlin side

```kotlin
class NativeBridge {
    companion object {
        init { System.loadLibrary("smart_explorer") } // matches the cdylib name cargo-ndk outputs
    }
    external fun dispatch(requestJson: String): String
}
```
`external fun` + `System.loadLibrary(<lib name without "lib" prefix / ".so" suffix>)` is the whole
Kotlin-side contract; no codegen needed (unlike UniFFI).

### 2.4 panic handling at the FFI boundary — `panic = "unwind"` vs `"abort"`

| Fact | Detail |
|---|---|
| Current desktop crate setting | `native/Cargo.toml:190` sets `panic = "unwind"` in `[profile.release]`, with an explicit comment that Dokany's FFI callback guards rely on unwinding to convert a panic into fail-stop mount teardown (`native/Cargo.toml:187-190`). This is a **Windows-Dokany-specific reason** that does not apply to the Android target (no Dokany on Android per the planning brief). |
| Why it matters for JNI specifically | Since Rust 1.81, **a panic that unwinds out of an `extern "C"` (or `extern "system"`) function aborts the process** — every `Java_...` native method is such a function, so an uncaught `unwrap()`/`expect()`/`panic!()` inside one kills the whole Android app with `SIGABRT` rather than failing just that call. Unwinding across the Rust↔JNI language boundary is itself **undefined behavior** if it isn't stopped before crossing. [Rust internals: unhandled panics in FFI](https://internals.rust-lang.org/t/unhandled-panics-in-rust-vs-in-ffi/17981), observed fix pattern in [aw-server-rust PR #681](https://github.com/ActivityWatch/aw-server-rust/pull/681) |
| Required mitigation | Wrap **every** exported `extern "system" fn Java_...` body in `std::panic::catch_unwind(|| { ... })` and convert any caught panic into a JNI exception (`env.throw_new(...)`) instead of letting it propagate — "every mature binding layer (PyO3, cxx, jni-rs) does this" at its own boundary; a hand-written JNI layer must do it explicitly itself. `jni-rs`'s own internal helpers (e.g. in its newer 0.22 `.resolve::<...>()` pattern) already wrap closures with `catch_unwind`; with 0.21's more manual style the project must add this wrapping itself around `dispatch_command`. |
| Cargo profile implication | `panic = "abort"` in the Android release profile would make this worse (an unwind-based `catch_unwind` cannot catch anything if unwinding is compiled out — a panic under `panic = "abort"` terminates immediately, full stop). **`panic = "unwind"` is required for `catch_unwind` at the JNI boundary to work at all**, independent of the Dokany-specific reason already in the comment. This means the existing `panic = "unwind"` profile setting is also correct for the Android target, for a different (JNI-boundary) reason — worth noting in the profile's comment if/when an Android target section is added. |

## 3. Logging (`android_logger` + `log`)

| Item | Value (checked 2026-09-25) |
|---|---|
| Crates | `log = "0.4"` (facade, cross-platform, likely already usable since `smart_explorer` has no logging dep listed in `native/Cargo.toml` currently — this would be new) + `android_logger = "0.15"` (target-specific, Android-only sink to Logcat) |
| Wiring | Add `android_logger` under a `[target.'cfg(target_os = "android")'.dependencies]` section (parallel to the existing `cfg(windows)` / `cfg(target_os = "linux")` sections at `native/Cargo.toml:108,168`) so it never touches the Windows/Linux desktop dependency graph. |
| Init pattern | ```rust
#[cfg(target_os = "android")]
fn init_logging() {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("smart_explorer"),
    );
}
``` Called once, e.g. from a `JNI_OnLoad` or the first `dispatch` call (guarded so it only runs once). |
| Source | [android_logger crate](https://crates.io/crates/android_logger), [docs.rs android_logger](https://docs.rs/android_logger) |

## 4. `core/` vs Android target_os note (grounding for the planning brief's claim)

Confirmed from `native/Cargo.toml`: the crate currently branches dependencies on `cfg(windows)`
(`native/Cargo.toml:108`), `cfg(not(windows))` (`:164`), and `cfg(target_os = "linux")` (`:168`) —
there is **no existing `cfg(target_os = "android")` branch**. Per the planning brief (not
independently re-verified against the Rust reference in this read-only pass, since that is a
language-spec fact rather than a web-research item under this assignment's scope): Android targets
report `target_os = "android"` and `target_family = "unix"`, so any current `cfg(target_os =
"linux")` dependency block (`russh`'s `zbus`/systemd D-Bus usage at `native/Cargo.toml:168-171`)
would **not** be compiled for an Android build — a new `cfg(target_os = "android")` (or a shared
`cfg(any(target_os = "linux", target_os = "android"))` block, chosen per-dependency) would be
needed wherever Linux-only logic must also run on Android.

---

## 5. UniFFI vs `jni` crate — summary comparison

| Dimension | UniFFI (0.32.2) | `jni` crate (0.21.x recommended, or 0.22.4 latest) |
|---|---|---|
| Boilerplate | Low — proc-macro/UDL + generated Kotlin | High — every exported fn hand-written, plus manual `catch_unwind` wrapping |
| Runtime dependency added on Android | JNA (`net.java.dev.jna:jna`, `@aar`) + its attach/detach-per-call and startup-signature-check overhead | None beyond the JVM's own JNI — direct calls |
| API stability risk | Pre-1.0, "more advanced things might break" across upgrades | 0.21→0.22 already shows this is not hypothetical — a real breaking rewrite happened within the crate's own history |
| Fit for a single narrow JSON-command surface (as sketched in the assignment) | Arguably heavier than needed — UniFFI shines for a **wide**, strongly-typed multi-function API surface | A **natural fit** — one or two hand-written `extern "system" fn` entry points exchanging JSON strings is exactly the "smallest surface" `jni`-crate use case, and reuses the crate's existing `serde_json` dependency (`native/Cargo.toml:61`) |
| Async/callback support | Built-in via UniFFI's Kotlin coroutine glue (needs `kotlinx-coroutines-core`) | Manual — background work on the Rust side (e.g. via existing `rayon`/threads) reporting back through a `JavaVM::attach_current_thread` callback into a Kotlin listener/`WorkManager` API |

### Open items for the main agent
- No independent primary-source check was done inside this pass for the `target_os`/`target_family`
  claim about Android in the planning brief (Rust reference/platform-support docs) — worth a quick
  confirmation before relying on it for `cfg` design, since this assignment's read surface was
  web + `native/Cargo.toml` only.
- `jni` 0.22.x's exact `.resolve::<ThrowRuntimeExAndDefault>()` error-mapping helper API was only
  partially captured (one crate-doc fetch); if 0.22.x is chosen over 0.21.x, re-fetch
  `docs.rs/jni/0.22.4` in full before writing the real bridge code.
- UniFFI's JNA minimum version differs slightly between two fetched sources (5.12.0 in the official
  Gradle-integration doc vs. 5.19.1 cited by a secondary source describing a newer plugin) — re-check
  the exact pinned version if UniFFI is chosen.
