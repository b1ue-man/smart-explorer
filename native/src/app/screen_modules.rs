//! Desktop-app files of strand S4 (screen, plan 2026-10-17 „Rechte, se
//! Desktop, Tresore“). `app/mod.rs` includes this file once; later S4
//! packages declare their app files here instead of in `app/mod.rs`.

// SCR-M2: the indicator window of `--screen-indicator` (V9 start kind
// `ScreenIndicator`, wired in `run_gui`).
#[path = "core/screen_indicator_ui.rs"]
mod screen_indicator_ui;
pub(crate) use screen_indicator_ui::run_screen_indicator;
// SCR-M2: tab 9 „Bildschirm“ of „Geräte & Freigaben“.
#[path = "core/share_screen_tab_ui.rs"]
pub(crate) mod share_screen_tab_ui;
