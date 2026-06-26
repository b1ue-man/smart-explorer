#[derive(Clone)]
pub struct DriveInfo {
    pub letter: String,
    pub label: String,
    pub serial: String,
}

pub(crate) struct DaemonInstanceGuard;

pub(crate) fn removable_drives() -> Vec<DriveInfo> {
    Vec::new()
}

pub(crate) fn battery_saver_on() -> bool {
    false
}

pub(crate) fn on_metered_network() -> bool {
    false
}

pub(crate) fn run_shell_command(cmd: &str) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("sh").args(["-c", cmd]).status()
}

pub(crate) fn acquire_daemon_instance_guard(
    _timeout: std::time::Duration,
) -> Option<DaemonInstanceGuard> {
    Some(DaemonInstanceGuard)
}
