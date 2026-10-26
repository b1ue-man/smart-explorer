//! Desktop-app files of strand S3 (vaults) of plan 2026-10-17 „Rechte, se
//! Desktop, Tresore“. `app/mod.rs` includes this file once (R7); later S3
//! packages declare their app files here instead of in `app/mod.rs`.

// VLT-M6: opening vault places (C7.2, K05 (1)).
#[path = "os/shared/vault_open.rs"]
pub(crate) mod vault_open;

// VLT-M10 (U29): the vault windows of the desktop. The calculation models
// are free of `egui`; the windows keep their state in a `static` each and
// the main window drives them from `ui_sidebar_vaults` (`vault_glue.rs`).
#[path = "core/vault_actions_ui.rs"]
pub(crate) mod vault_actions_ui;
#[path = "core/vault_add_ui.rs"]
pub(crate) mod vault_add_ui;
#[path = "core/vault_check_ui.rs"]
pub(crate) mod vault_check_ui;
#[path = "core/vault_compare_detail_ui.rs"]
pub(crate) mod vault_compare_detail_ui;
#[path = "core/vault_compare_list_ui.rs"]
pub(crate) mod vault_compare_list_ui;
#[cfg(test)]
#[path = "core/vault_compare_tests.rs"]
mod vault_compare_tests;
#[path = "core/vault_compare_ui.rs"]
pub(crate) mod vault_compare_ui;
#[path = "core/vault_create_ui.rs"]
pub(crate) mod vault_create_ui;
#[path = "core/vault_feed.rs"]
pub(crate) mod vault_feed;
#[path = "core/vault_glue.rs"]
pub(crate) mod vault_glue;
#[cfg(test)]
#[path = "core/vault_gui_tests.rs"]
mod vault_gui_tests;
#[cfg(test)]
#[path = "core/vault_manage_tests.rs"]
mod vault_manage_tests;
#[path = "core/vault_models.rs"]
pub(crate) mod vault_models;
#[path = "core/vault_models_compare.rs"]
pub(crate) mod vault_models_compare;
#[path = "core/vault_models_manage.rs"]
pub(crate) mod vault_models_manage;
#[path = "core/vault_models_sync.rs"]
pub(crate) mod vault_models_sync;
#[cfg(test)]
#[path = "core/vault_models_tests.rs"]
mod vault_models_tests;
#[path = "core/vault_settings_pages_ui.rs"]
pub(crate) mod vault_settings_pages_ui;
#[path = "core/vault_settings_ui.rs"]
pub(crate) mod vault_settings_ui;
#[path = "core/vault_open_policy_ui.rs"]
pub(crate) mod vault_open_policy_ui;
#[path = "core/vault_sidebar_ui.rs"]
pub(crate) mod vault_sidebar_ui;
#[path = "core/vault_sync_editor_ui.rs"]
pub(crate) mod vault_sync_editor_ui;
#[cfg(test)]
#[path = "core/vault_sync_tests.rs"]
mod vault_sync_tests;
#[path = "core/vault_ui_settings.rs"]
pub(crate) mod vault_ui_settings;
#[path = "core/vault_unlock_ui.rs"]
pub(crate) mod vault_unlock_ui;
