//! Modules of the compact connect and share flow (plan 2026-10-31 §F,
//! F1–F7): `share/mod.rs` declares this file once and re-exports its
//! modules (`pub(crate) use verbinden_modules::*`).
#[path = "core/connection_rights.rs"]
pub(crate) mod connection_rights;
#[path = "os/shared/connection_rights_actions.rs"]
pub(crate) mod connection_rights_actions;
#[path = "core/connection_rights_apply.rs"]
pub(crate) mod connection_rights_apply;
#[path = "core/connection_rights_exec.rs"]
pub(crate) mod connection_rights_exec;
#[path = "core/exec_follow_up.rs"]
pub(crate) mod exec_follow_up;
