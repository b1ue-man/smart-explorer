fn main() {
    if std::env::args().any(|arg| arg == "--sync-daemon") {
        smart_explorer::daemon::run_daemon();
        return;
    }
    std::process::exit(smart_explorer::cli::run());
}
