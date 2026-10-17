//! Desktop-app files of strand S1 (access rights, plan 2026-10-17
//! „Rechte, se Desktop, Tresore“). `app/mod.rs` includes this file once;
//! later S1 packages declare their app files here instead of in
//! `app/mod.rs`.

// PRM-M3: the confirmation window of `--access-consent <ID>` (V9 start kind
// `AccessConsent`, wired in `run_gui`).
#[path = "core/access_consent_ui.rs"]
mod access_consent_ui;
pub(crate) use access_consent_ui::run_access_consent;
