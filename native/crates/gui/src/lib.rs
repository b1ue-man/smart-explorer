//! The desktop GUI of Smart Explorer: the explorer window (`app`), se Desktop
//! (`desktop`) and their Windows shell helpers (plan
//! docs/plaene/2026-11-01-idealzustand, D13). It builds on the engine, whose
//! modules it re-exports, so `crate::<module>` paths reach them; a change here
//! recompiles only this crate and the ones above it (CLI, programs).

pub use smart_explorer_engine::*;
// The programs name the result type of `run_gui` through it.
pub use eframe;

pub mod app;
pub mod desktop;
pub mod dragout;
pub mod icons;
pub mod quickshare;
#[cfg(windows)]
pub mod shell_clipboard;
#[cfg(windows)]
pub mod shell_menu;
#[cfg(windows)]
pub mod shell_register;
#[cfg(windows)]
pub mod virtual_clipboard;

pub fn run_gui() -> eframe::Result<()> {
    let raw_args: Vec<_> = std::env::args_os().skip(1).collect();
    if let Some(result) = run_login_service_if_requested(&raw_args) {
        if let Err(error) = result {
            eprintln!("Login service: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    install_panic_logger();
    // The headless read helper must precede ordinary GUI/daemon side effects.
    if let Some(result) = local_access::run_helper_if_requested(&raw_args) {
        if let Err(error) = result {
            eprintln!("Lesehelfer: {error}");
        }
        return Ok(());
    }
    if let Some(result) = share::run_exec_supervisor_if_requested(&raw_args) {
        result.unwrap_or_else(|error| panic!("remote-exec supervisor failed: {error}"));
        return Ok(());
    }

    if raw_args.len() == 1 && raw_args[0] == std::ffi::OsStr::new("--sync-guardian") {
        daemon::run_guardian();
        return Ok(());
    }

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--sync-daemon") {
        daemon::note_daemon_start_args(&raw_args);
        daemon::run_daemon();
        return Ok(());
    }

    // V9 background kinds of se Desktop, before the update confirmation.
    if let Some(code) = desktop::run_if_requested(&raw_args) {
        std::process::exit(code);
    }
    // V9 background kind of S4, also before the update confirmation: the
    // indicator window of running screen sessions, started by the worker.
    if let Ok(app::start_request::StartRequest::ScreenIndicator) =
        app::start_request::parse(&raw_args)
    {
        return app::run_screen_indicator();
    }
    // SCR-M3: the screen host process of one session (`--screen-host-worker`).
    if let Some(code) = run_screen_host_worker_if_requested(&raw_args) {
        std::process::exit(code);
    }

    let just_updated = args.iter().any(|a| a == "--updated");
    let startup_ack_pending =
        updater::capture_update_startup_ack(just_updated).unwrap_or_else(|error| panic!("{error}"));
    if !startup_ack_pending {
        updater::cleanup_old_binaries();
        updater::archive_current_version();
    }

    #[cfg(windows)]
    if args.iter().any(|a| a == "--unregister") {
        shell_register::unregister_all();
        return Ok(());
    }

    // V9 window kinds after the updater steps, first the S1 consent window.
    if let Ok(app::start_request::StartRequest::AccessConsent { id }) =
        app::start_request::parse(&raw_args)
    {
        return app::run_access_consent(id);
    }

    // SCR-M7: the screen viewer of one device and the group view of a room.
    match app::start_request::parse(&raw_args) {
        Ok(app::start_request::StartRequest::ScreenView { target_json }) => {
            std::process::exit(app::run_screen_view(&target_json));
        }
        Ok(app::start_request::StartRequest::ScreenGrid { room }) => {
            std::process::exit(app::run_screen_grid(room.as_deref()));
        }
        _ => {}
    }

    #[cfg(windows)]
    shell_register::cleanup_stale_default_manager();

    // The explorer window, optionally at a location or page (S5); the value of
    // a switch never becomes the start path.
    use app::start_request::StartRequest;
    let start = match app::start_request::parse(&raw_args) {
        Ok(
            start @ (StartRequest::Gui { .. }
            | StartRequest::OpenLocation { .. }
            | StartRequest::OpenPage { .. }),
        ) => start,
        Ok(other) => {
            let switch = other.to_args().into_iter().next().unwrap_or_default();
            eprintln!(
                "smart_explorer: {} ist in diesem Programm nicht verfügbar",
                switch.to_string_lossy()
            );
            std::process::exit(2);
        }
        Err(error) => {
            eprintln!("smart_explorer: {error}");
            std::process::exit(2);
        }
    };
    desktop::resume_on_gui_start();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([900.0, 600.0])
            .with_icon(window_icon())
            .with_title("Smart Explorer"),
        ..Default::default()
    };

    #[cfg(windows)]
    shell_menu::init_com();

    app::screen_render_app::run_native(
        "Smart Explorer",
        options,
        Box::new(|cc| {
            cc.egui_ctx.data_mut(|data| {
                data.insert_temp(
                    eframe::egui::Id::new("screen_renderer_wgpu"),
                    cc.wgpu_render_state.is_some(),
                )
            });
            let app = match start {
                StartRequest::Gui { start_path } => app::App::new(just_updated, start_path),
                request => app::App::new_with_start(just_updated, request),
            };
            app.configure_appearance(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

fn window_icon() -> eframe::egui::IconData {
    eframe::egui::IconData {
        rgba: include_bytes!("../../../assets/smart-explorer-icon-256.rgba").to_vec(),
        width: 256,
        height: 256,
    }
}
