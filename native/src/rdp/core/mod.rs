//! Portable RDP inventory and admission; no platform discovery in this layer.
mod types;
mod admission;
pub(crate) use self::types::*;
pub(crate) use self::admission::{AuthenticatedProbe, StartRequest};

#[cfg(test)]
mod batches_admission_task_tests;
