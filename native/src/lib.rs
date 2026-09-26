// Android builds only the core library behind the Kotlin UI (no egui app, no
// CLI). Helpers that only the desktop GUI/CLI reach are therefore unused there;
// the desktop builds keep the full dead-code and unused-import lints.
#![cfg_attr(target_os = "android", allow(dead_code, unused_imports))]

pub mod agent;
pub mod agent_proto;
pub mod analytics;
#[cfg(any(target_os = "android", all(unix, test)))]
pub(crate) mod android_fs;
#[cfg(not(target_os = "android"))]
pub mod app;
pub mod apptrash;
pub mod autostart;
pub mod bisync;
#[cfg(not(target_os = "android"))]
pub mod cli;
pub mod cloud;
pub mod connect;
pub mod copy;
pub mod creds;
pub mod daemon;
pub mod dragout;
pub mod filter;
pub mod folder_index;
pub mod format;
pub mod ftp;
pub mod gdrive;
#[cfg(not(target_os = "android"))]
pub mod icons;
pub mod linemerge;
mod local_access;
#[cfg(any(target_os = "android", all(unix, test)))]
pub mod mobile;
pub mod mount;
pub mod net;
pub mod quickshare;
pub mod rscan;
pub mod scanner;
pub mod sftp;
pub mod share;
#[cfg(windows)]
pub mod shell_clipboard;
#[cfg(windows)]
pub mod shell_menu;
#[cfg(windows)]
pub mod shell_register;
pub mod smb;
pub mod support_dirs;
pub mod sync;
pub mod syncjobs;
pub mod transfer;
pub mod types;
pub mod updater;
pub mod vfs;
#[cfg(windows)]
pub mod virtual_clipboard;
pub mod webdav;
pub mod zipfs;

#[cfg(not(target_os = "android"))]
pub fn run_gui() -> eframe::Result<()> {
    let raw_args: Vec<_> = std::env::args_os().skip(1).collect();
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

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--sync-daemon") {
        daemon::run_daemon();
        return Ok(());
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

    #[cfg(windows)]
    shell_register::cleanup_stale_default_manager();

    let initial_path = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .map(std::path::PathBuf::from);

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

    eframe::run_native(
        "Smart Explorer",
        options,
        Box::new(|cc| {
            let app = app::App::new(just_updated, initial_path);
            app.configure_appearance(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(not(target_os = "android"))]
fn window_icon() -> eframe::egui::IconData {
    eframe::egui::IconData {
        rgba: include_bytes!("../assets/smart-explorer-icon-256.rgba").to_vec(),
        width: 256,
        height: 256,
    }
}

/// Appends every panic (thread name, message, backtrace) to the app data
/// `crash.log`, then runs the previously installed hook. The desktop GUI installs
/// it at startup; an embedding host (Android facade) calls it once after its data
/// directories are known. Repeated calls keep the first installation, so the log
/// never receives duplicate entries.
pub fn install_panic_logger() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(install_panic_logger_hook);
}

fn install_panic_logger_hook() {
    use std::io::Write;
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let log_path = crate::support_dirs::app_data_file("crash.log");
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let thread = std::thread::current();
            let _ = writeln!(
                f,
                "\n=== PANIC {} ({}) ===\n{}\nbacktrace:",
                ts,
                thread.name().unwrap_or("<unnamed>"),
                info
            );
            let bt = std::backtrace::Backtrace::force_capture();
            let _ = writeln!(f, "{}", bt);
        }
        default_hook(info);
    }));
}
