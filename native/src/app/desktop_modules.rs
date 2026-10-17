//! Desktop-app files of S5 (se Desktop) and of the shared start and update
//! contracts V8/V9. `app/mod.rs` includes this file once; later S5 packages
//! declare their app files here instead of in `app/mod.rs`.

// V9: start switches of the GUI program, one decision for `run_gui`.
#[path = "core/start_request.rs"]
pub(crate) mod start_request;
// V8/DSK-M2: every egui window closes on a new update-stop request.
#[path = "core/update_stop_ui.rs"]
pub(crate) mod update_stop_ui;
