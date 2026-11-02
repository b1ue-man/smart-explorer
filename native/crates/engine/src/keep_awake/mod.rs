//! Keeping the system awake while long work runs (RV1, contract V4): sync
//! runs, serving another device's analysis or transfer, and own tasks on
//! another device.
//!
//! - Windows: a power request (system and execution required) per reason and
//!   power throttling (EcoQoS) off for the process while any hold lives.
//! - Linux: a logind inhibitor `idle:sleep` (mode `block`) over the system
//!   D-Bus (`zbus`); the lock ends with the process. Without logind nothing is
//!   inhibited and `status().unavailable` says why.
//! - Android: the existing CPU-hold hook (`share::power::request_hold`, which
//!   the app turns into a partial wake lock), renewed while holds live and at
//!   once when the app enters low-power operation.
//!
//! Users: the background worker and the desktop and Android sync runs
//! (`SyncRun`), the Share host while foreign streams run (`PeerService`), and
//! analysis or duplicate tasks against another device (`RemoteTask`).

#[path = "os/shared/holds.rs"]
mod holds;
#[cfg(target_os = "android")]
#[path = "os/android.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "os/linux_os.rs"]
mod platform;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[path = "core/types.rs"]
mod types;

pub use holds::{hold, status, KeepAwake};
pub use types::{KeepAwakeStatus, Reason};
