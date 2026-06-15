// Hide console window on Windows release builds
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod app;
mod copy;
mod creds;
mod filter;
mod ftp;
mod folder_index;
mod format;
mod icons;
mod net;
mod rscan;
mod scanner;
mod sftp;
#[cfg(windows)]
mod shell_clipboard;
#[cfg(windows)]
mod shell_register;
#[cfg(windows)]
mod shell_menu;
mod types;
mod updater;
mod vfs;
#[cfg(windows)]
mod virtual_clipboard;

fn main() -> eframe::Result<()> {
    install_panic_logger();

    // Remove the *_old.exe leftover from a previous self-update and note
    // whether we were just relaunched by one.
    updater::cleanup_old_binaries();
    // Preserve this version so it's available to roll back to after future
    // updates (archives accumulate from here forward).
    updater::archive_current_version();
    let args: Vec<String> = std::env::args().collect();

    // Uninstaller hook: undo all shell integration (reversible by design) and
    // exit without opening a window, so removing the app can't leave a folder
    // handler pointing at a deleted exe.
    #[cfg(windows)]
    if args.iter().any(|a| a == "--unregister") {
        shell_register::unregister_all();
        return Ok(());
    }

    // Remove the never-working 0.3.4 default-manager override keys if present.
    #[cfg(windows)]
    shell_register::cleanup_stale_default_manager();

    let just_updated = args.iter().any(|a| a == "--updated");
    // First non-flag argument is a path to open (folder double-click, the
    // "Open in Smart Explorer" verb, or default-file-manager handoff). Files
    // are handled by opening their parent folder.
    let initial_path = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("Smart Explorer"),
        ..Default::default()
    };
    // Init COM apartment-threaded for shell IContextMenu calls.
    #[cfg(windows)]
    shell_menu::init_com();

    eframe::run_native(
        "Smart Explorer",
        options,
        Box::new(|cc| {
            // Enable a visually distinct dark theme
            cc.egui_ctx.set_visuals(eframe::egui::Visuals::dark());
            Ok(Box::new(app::App::new(just_updated, initial_path)))
        }),
    )
}

/// Capture panics from any thread to %APPDATA%\smart_explorer\crash.log so we
/// can diagnose silent crashes / freezes after the fact. Each panic appends a
/// timestamped block. Process still exits afterwards on a main-thread panic.
fn install_panic_logger() {
    use std::io::Write;
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let log_path: std::path::PathBuf = std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("smart_explorer")
            .join("crash.log");
        let _ = std::fs::create_dir_all(log_path.parent().unwrap());
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
