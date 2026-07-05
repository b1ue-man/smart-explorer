pub mod agent;
pub mod agent_proto;
pub mod analytics;
pub mod app;
pub mod autostart;
pub mod bisync;
pub mod cli;
pub mod cloud;
pub mod connect;
pub mod copy;
pub mod creds;
pub mod daemon;
#[cfg(windows)]
pub mod dragout;
pub mod filter;
pub mod folder_index;
pub mod format;
pub mod ftp;
pub mod gdrive;
pub mod icons;
pub mod linemerge;
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
pub mod support_dirs;
pub mod sync;
pub mod syncjobs;
pub mod types;
pub mod updater;
pub mod vfs;
#[cfg(windows)]
pub mod virtual_clipboard;
pub mod webdav;
pub mod zipfs;

pub fn run_gui() -> eframe::Result<()> {
    install_panic_logger();

    updater::cleanup_old_binaries();
    updater::archive_current_version();
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--apply-update") {
        updater::run_apply_worker(&args);
        return Ok(());
    }

    if args.iter().any(|a| a == "--sync-daemon") {
        daemon::run_daemon();
        return Ok(());
    }

    #[cfg(windows)]
    if args.iter().any(|a| a == "--unregister") {
        shell_register::unregister_all();
        return Ok(());
    }

    #[cfg(windows)]
    shell_register::cleanup_stale_default_manager();

    let just_updated = args.iter().any(|a| a == "--updated");
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
