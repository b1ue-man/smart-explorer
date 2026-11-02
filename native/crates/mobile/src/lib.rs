//! The Android facade of Smart Explorer behind the Kotlin UI (plan
//! docs/plaene/2026-11-01-idealzustand, D13). It builds on the engine, whose
//! modules it re-exports, so `crate::<module>` paths reach them.
// Helpers that only the desktop GUI/CLI reach are unused on Android.
#![cfg_attr(target_os = "android", allow(dead_code, unused_imports))]

pub use smart_explorer_engine::*;

#[cfg(all(feature = "android-instrumentation-fixtures", not(debug_assertions)))]
compile_error!("Android image fixtures are allowed only in a debug instrumentation build");

#[cfg(any(target_os = "android", all(unix, test)))]
pub mod mobile;
