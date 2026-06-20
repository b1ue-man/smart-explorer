// Hide console window on Windows release builds
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod agent;
mod agent_proto;
mod analytics;
mod app;
mod autostart;
mod bisync;
mod cloud;
mod connect;
mod copy;
mod creds;
mod daemon;
#[cfg(windows)]
mod dragout;
mod filter;
mod folder_index;
mod format;
mod ftp;
mod gdrive;
mod icons;
mod linemerge;
mod net;
mod quickshare;
mod rscan;
mod scanner;
mod sftp;
mod share;
#[cfg(windows)]
mod shell_clipboard;
#[cfg(windows)]
mod shell_menu;
#[cfg(windows)]
mod shell_register;
mod support_dirs;
mod sync;
mod syncjobs;
mod types;
mod updater;
mod vfs;
#[cfg(windows)]
mod virtual_clipboard;
mod webdav;
mod zipfs;

fn main() -> eframe::Result<()> {
    install_panic_logger();

    // Remove the *_old.exe leftover from a previous self-update and note
    // whether we were just relaunched by one.
    updater::cleanup_old_binaries();
    // Preserve this version so it's available to roll back to after future
    // updates (archives accumulate from here forward).
    updater::archive_current_version();
    let args: Vec<String> = std::env::args().collect();

    // Update worker: a temp copy of the new binary, launched by a failed
    // in-place swap. Waits for the parent to exit, replaces the target exe,
    // relaunches it. Opens no window.
    if args.iter().any(|a| a == "--apply-update") {
        updater::run_apply_worker(&args);
        return Ok(());
    }

    // Headless background sync (logon autostart). Runs the daemon loop and
    // never opens a window. The same exe, so self-update keeps it current.
    if args.iter().any(|a| a == "--sync-daemon") {
        daemon::run_daemon();
        return Ok(());
    }

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
            // Open at a sensible, on-screen size and maximize on the first
            // painted frame (see App::update) rather than via the builder's
            // `maximized`: that showed a white, default-sized window for a beat
            // and then jumped to maximized — the "flashbang" the user saw. This
            // also fixes the earlier partly-off-screen placement. inner_size is
            // the restore (un-maximized) size.
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([900.0, 600.0])
            .with_icon(window_icon())
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

fn window_icon() -> eframe::egui::IconData {
    eframe::egui::IconData {
        rgba: include_bytes!("../assets/smart-explorer-icon-256.rgba").to_vec(),
        width: 256,
        height: 256,
    }
}

/// Capture panics from any thread to the app data crash log so we can diagnose
/// silent crashes / freezes after the fact. Each panic appends a timestamped
/// block. Process still exits afterwards on a main-thread panic.
fn install_panic_logger() {
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
