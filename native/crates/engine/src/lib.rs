//! The engine of Smart Explorer: every module the desktop GUI, the CLI and the
//! Android facade build on (plan docs/plaene/2026-11-01-idealzustand, D13).
//! `smart_explorer_gui`, `smart_explorer_cli` and `smart_explorer_mobile`
//! re-export it, so their `crate::<module>` paths reach these modules.
// Android builds only the core library behind the Kotlin UI (no egui app, no
// CLI). Helpers that only the desktop GUI/CLI reach are therefore unused there;
// the desktop builds keep the full dead-code and unused-import lints.
#![cfg_attr(target_os = "android", allow(dead_code, unused_imports))]

#[cfg(all(feature = "android-instrumentation-fixtures", not(debug_assertions)))]
compile_error!("Android image fixtures are allowed only in a debug instrumentation build");

pub mod agent;
pub mod agent_proto;
pub mod analytics;
#[cfg(unix)]
pub mod android_fs;
pub mod apps;
pub mod apptrash;
pub mod autostart;
pub mod bisync;
pub mod cloud;
pub mod connect;
pub mod copy;
pub mod creds;
pub mod cryptomator;
pub mod daemon;
pub mod filter;
pub mod folder_index;
pub mod format;
pub mod ftp;
pub mod gdrive;
pub mod host;
pub mod host_trash;
pub mod keep_awake;
pub mod licenses;
pub mod linemerge;
pub mod local_access;
pub mod login;
#[cfg(not(target_os = "android"))]
pub mod login_entry;
#[cfg(not(target_os = "android"))]
pub use login_entry::run_login_service_if_requested;
pub mod mcp_launcher;
pub mod mount;
pub mod net;
/// Ports: lower layers reach their higher implementations through the
/// linker (D13); `declare_port!` and `provide_port!` are defined there.
pub mod ports;
pub mod rdp;
pub mod remote_window;
pub mod rscan;
pub mod scanner;
pub mod screen;
pub use screen::os::host_worker_main::run_screen_host_worker_if_requested;
pub mod sftp;
pub mod share;
pub mod smb;
pub mod source_media;
pub mod support_dirs;
pub mod sync;
pub mod syncjobs;
pub mod transfer;
pub mod types;
pub mod updater;
pub mod vault;
pub mod vault_error;
pub mod vfs;
pub mod watch;
pub mod webdav;
pub mod worker_log;
pub mod zipfs;

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

pub mod notify_desktop;
