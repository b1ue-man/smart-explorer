//! Modules of the access rights (strand S1 of the plan 2026-10-17
//! „Rechte, se Desktop, Tresore“): `share/mod.rs` declares this file once
//! and re-exports its modules (`pub(crate) use access_modules::*`), so the
//! strand adds its modules here instead of in the hub.
#[path = "core/access_authorize.rs"]
pub(crate) mod access_authorize;
#[path = "core/access_catalog.rs"]
pub(crate) mod access_catalog;
#[path = "core/access_grant.rs"]
pub(crate) mod access_grant;
#[path = "core/access_grant_runtime.rs"]
pub(crate) mod access_grant_runtime;
#[path = "core/access_grant_wire.rs"]
pub(crate) mod access_grant_wire;
#[path = "core/exec_access_view.rs"]
pub(crate) mod exec_access_view;
#[path = "core/exec_client_messages.rs"]
pub(crate) mod exec_client_messages;
#[path = "core/relation_proof.rs"]
pub(crate) mod relation_proof;
