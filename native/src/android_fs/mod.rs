//! Android filesystem adapters.
//!
//! Shared storage on Android 11+ is served by the MediaProvider FUSE daemon,
//! which lacks some Linux primitives the desktop adapters rely on. The module
//! is compiled for Android, and for unix test builds so the chain also runs on
//! the Linux host; the desktop adapters never call it.
#[path = "os/rename.rs"]
mod rename;
#[cfg(test)]
#[path = "os/rename_tests.rs"]
mod rename_tests;

pub(crate) use rename::rename_no_replace;
