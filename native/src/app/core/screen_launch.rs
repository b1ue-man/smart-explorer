//! Start of the viewer windows (SCR-M7, U25-2): `smart_explorer --screen-view
//! <ziel-json>` and `--screen-grid [--room <id>]` run the viewer window until
//! it closes and give its exit code (0, or 1 after an error window); and the
//! other direction, the pages of „Geräte & Freigaben“ and `se screen view`
//! start such a window as a process of its own (V5). The window arms the
//! update stop (V8) before it opens, and closes through `close_requested` in
//! every frame.
use eframe::egui;

use super::screen_devices::GridScope;
use super::screen_prefs::ViewerPrefs;
use super::screen_target::ScreenTarget;
use super::screen_viewer_app::{wgpu_configuration, ScreenViewerApp, Start};
use crate::app::start_request::StartRequest;

/// The application ID of the viewer window (Wayland app ID, window class).
pub(crate) const APP_ID: &str = "app.smartexplorer.screen";

fn exit_code(result: eframe::Result<()>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Smart Explorer: Bildschirmfenster: {error}");
            1
        }
    }
}

fn viewport(title: &str) -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_title(title)
        .with_app_id(APP_ID)
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([640.0, 400.0])
        .with_icon(crate::window_icon())
}

/// Opens the viewer window with wgpu, and with the CPU path (glow) when the
/// window cannot get a wgpu surface.
fn run_viewer(start: Start, title: &str) -> i32 {
    let prefs = ViewerPrefs::load();
    let wgpu = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: wgpu_configuration(prefs.low_latency),
        viewport: viewport(title),
        ..Default::default()
    };
    let (first, second) = (start.clone(), start);
    let (first_prefs, second_prefs) = (prefs.clone(), prefs);
    let result = eframe::run_native(
        APP_ID,
        wgpu,
        Box::new(move |cc| Ok(Box::new(ScreenViewerApp::new(cc, first, first_prefs)))),
    );
    if result.is_ok() {
        return 0;
    }
    eprintln!("Smart Explorer: wgpu nicht verfügbar, Bild wird auf der CPU gezeichnet");
    let glow = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: viewport(title),
        ..Default::default()
    };
    exit_code(eframe::run_native(
        APP_ID,
        glow,
        Box::new(move |cc| Ok(Box::new(ScreenViewerApp::new(cc, second, second_prefs)))),
    ))
}

/// `--screen-view <ziel-json>`.
pub(crate) fn run_screen_view(target_json: &str) -> i32 {
    crate::app::update_stop_ui::arm_window();
    match ScreenTarget::parse(target_json) {
        Ok(target) if target.source.as_ref().and_then(crate::screen::core::family_protocol::selector).is_some()
            => super::family_viewer::run(target),
        Ok(target) => run_viewer(Start::View(target), "Bildschirm"),
        Err(message) => error_window(&message),
    }
}

/// `--screen-grid [--room <id>]`.
pub(crate) fn run_screen_grid(room: Option<&str>) -> i32 {
    crate::app::update_stop_ui::arm_window();
    let scope = room.map_or(GridScope::AllDevices, |room| GridScope::Room(room.into()));
    run_viewer(Start::Grid(scope), "Bildschirme")
}

/// The window of a start that cannot go on: the text and „Schließen“.
fn error_window(message: &str) -> i32 {
    struct ErrorWindow(String);
    impl eframe::App for ErrorWindow {
        fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            if crate::app::update_stop_ui::close_requested(ctx) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label(&self.0);
                ui.add_space(8.0);
                if ui.button("Schließen").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Bildschirm – Smart Explorer")
            .with_app_id(APP_ID)
            .with_inner_size([480.0, 160.0])
            .with_icon(crate::window_icon()),
        ..Default::default()
    };
    let text = message.to_string();
    let _ = eframe::run_native(
        APP_ID,
        options,
        Box::new(move |_cc| Ok(Box::new(ErrorWindow(text)))),
    );
    1
}

/// Starts a window of the GUI program as a process of its own.
fn spawn(request: &StartRequest) -> Result<(), String> {
    let gui = crate::desktop::os::launch::gui_binary()?;
    let args = crate::desktop::os::open_target::window_args(request);
    crate::desktop::os::open_target::spawn_detached(&gui, &args)
}

/// „Bildschirm öffnen“ / „Bildschirm steuern“ of a device.
pub(crate) fn open_view(target: &ScreenTarget) -> Result<(), String> {
    let target_json = serde_json::to_string(target).map_err(|error| error.to_string())?;
    spawn(&StartRequest::ScreenView { target_json })
}

/// „Bildschirme des Raums“ (`Some(room profile ID)`) and „Alle Geräte“.
pub(crate) fn open_grid(room: Option<&str>) -> Result<(), String> {
    spawn(&StartRequest::ScreenGrid {
        room: room.map(str::to_string),
    })
}

/// A button of the pages that starts the viewer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LaunchButton {
    pub label: &'static str,
    pub enabled: bool,
    /// Why it is off.
    pub hint: &'static str,
    /// „Bildschirm steuern“.
    pub control: bool,
}

const NO_DEVICE_ID: &str = "Die Kennung des Geräts ist noch nicht bekannt.";
const NOT_ACCEPTED: &str = "Das Gerät hat den Zugriff noch nicht angenommen.";

/// „Bildschirm öffnen“ and „Bildschirm steuern“ of a Direct device: on
/// when its device ID is known and it accepted this device. Whether it
/// shares its screen is up to the device; the viewer shows its answer.
pub(crate) fn device_buttons(device_id: Option<&str>, accepted: bool) -> [LaunchButton; 2] {
    let (enabled, hint) = match (device_id.is_some(), accepted) {
        (false, _) => (false, NO_DEVICE_ID),
        (true, false) => (false, NOT_ACCEPTED),
        (true, true) => (true, ""),
    };
    [
        LaunchButton {
            label: "Bildschirm öffnen",
            enabled,
            hint,
            control: false,
        },
        LaunchButton {
            label: "Bildschirm steuern",
            enabled,
            hint,
            control: true,
        },
    ]
}

/// „Bildschirme des Raums“.
pub(crate) fn room_button(joined: bool) -> LaunchButton {
    LaunchButton {
        label: "Bildschirme des Raums",
        enabled: joined,
        hint: "Der Raum ist verlassen.",
        control: false,
    }
}

/// „Alle Geräte“ of the tab „Bildschirm“.
pub(crate) fn all_devices_button() -> LaunchButton {
    LaunchButton {
        label: "Alle Geräte",
        enabled: true,
        hint: "",
        control: false,
    }
}

/// Draws `button`; whether it was clicked.
pub(crate) fn show_button(ui: &mut egui::Ui, button: &LaunchButton) -> bool {
    let response = ui.add_enabled(button.enabled, egui::Button::new(button.label));
    response.on_disabled_hover_text(button.hint).clicked()
}
