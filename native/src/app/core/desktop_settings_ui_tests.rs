//! Erhaltungsfall F13 (GUI harness `SMART_EXPLORER_GUI_TASK=1`, isolated app
//! data, one test thread): without se Desktop a GUI start and the settings
//! page "Integration" start no process and create nothing under
//! `<AD>/desktop/`.
use std::time::{Duration, Instant};

use eframe::egui;
use serde_json::{json, Value};

use super::{lock, Shared};
use crate::app::settings_ui::SettingsPage;
use crate::app::App;
use crate::desktop::core::state::State;
use crate::desktop::os::platform;
use crate::desktop::os::store::DesktopDir;

fn input() -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1200.0, 900.0),
        )),
        ..Default::default()
    }
}

#[test]
#[ignore = "run only through the isolated remote GUI task entrypoint"]
fn desktop_task_absent_starts_nothing() {
    let dir = DesktopDir::production();
    assert!(!dir.exists(), "precondition: se Desktop is absent");
    crate::desktop::resume_on_gui_start();
    let mut app = App::new_for_gui_task();
    app.open_settings(SettingsPage::Integration);
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(15);
    let state = loop {
        let _ = ctx.run(input(), |ctx| app.ui_settings(ctx));
        let shared: Option<Shared> =
            ctx.data_mut(|data| data.get_temp::<Shared>(egui::Id::new("se_desktop_settings")));
        let state =
            shared.and_then(|shared| lock(&shared).report.as_ref().map(|report| report.state));
        if let Some(state) = state {
            break state;
        }
        assert!(
            Instant::now() < deadline,
            "settings never showed se Desktop"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(state, State::Absent);
    // The resume thread of the GUI start has long finished.
    std::thread::sleep(Duration::from_millis(500));
    assert!(!dir.exists(), "<AD>/desktop/ appeared");
    assert!(!platform::instance::is_running(&dir));
    let status = crate::desktop::status_value();
    assert_eq!(status["state"], json!("absent"));
    assert_eq!(status["processes"]["shell_pid"], Value::Null);
    assert_eq!(status["residue"], json!([]));
    assert!(!dir.exists());
}
