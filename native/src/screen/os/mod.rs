//! Platform side of the screen strand (S4). SCR-M2 brings the parts every
//! platform shares: the test-picture backend and the indicator window of the
//! desktop worker. Capture, codecs and input of each platform follow under
//! `windows/`, `linux_os/`, `android/` and `native_ffi/` (SCR-M3 to SCR-M8),
//! selected here with `#[cfg(...)]`.

#[path = "shared/indicator.rs"]
pub(crate) mod indicator;
#[path = "shared/synthetic_backend.rs"]
pub(crate) mod synthetic_backend;
