//! Portable RDP inventory and admission; no platform discovery in this layer.
mod admission;
mod login;
mod types;
/// xrdp sign-in with a one-time se token: PAM line and token decision (P4).
pub mod xrdp_login;
pub use self::admission::{AuthenticatedProbe, StartRequest};
pub use self::login::*;
pub use self::types::*;

#[cfg(test)]
mod batches_admission_task_tests;
