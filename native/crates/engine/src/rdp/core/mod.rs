//! Portable RDP inventory and admission; no platform discovery in this layer.
mod admission;
mod types;
pub use self::admission::{AuthenticatedProbe, StartRequest};
pub use self::types::*;

#[cfg(test)]
mod batches_admission_task_tests;
