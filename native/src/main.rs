#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

fn main() -> eframe::Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if let Some(result) = smart_explorer::mount::run_host_if_requested(&arguments) {
        exit_mount_host(result);
    }
    smart_explorer::run_gui()
}

fn exit_mount_host(result: Result<(), String>) -> ! {
    match result {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            eprintln!("smart-explorer: internal mount host failed: {error}");
            std::process::exit(1)
        }
    }
}
