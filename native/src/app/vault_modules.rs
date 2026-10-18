//! Desktop-app files of strand S3 (vaults) of plan 2026-10-17 „Rechte, se
//! Desktop, Tresore“. `app/mod.rs` includes this file once (R7); later S3
//! packages declare their app files here instead of in `app/mod.rs`.

// VLT-M6: opening vault places (C7.2, K05 (1)).
#[path = "os/shared/vault_open.rs"]
pub(crate) mod vault_open;
