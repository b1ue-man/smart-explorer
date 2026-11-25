//! What `se` knows about the Exec host of a Share peer: its operating system,
//! learned from the provider of each Exec answer and kept in
//! `exec-peer-os.json` of the app data, so `connections` names it before
//! the next command; and the name of the shell an Exec result ran in.
use crate::share::exec_shell::HostFamily;
use crate::share::{ExecTerminal, ExecTerminalKind};
use serde_json::{Map, Value};
use std::path::Path;

const FILE: &str = "exec-peer-os.json";
/// Result name of a command that `argv` started without a shell.
pub(super) const NO_SHELL: &str = "none";

/// Remembers the OS of the peer at `endpoint` (`share://direct/<id>` or a
/// room device's `share://room/<room>/<device>`) from its Exec provider.
/// Best effort: a failed write only loses the hint.
pub(super) fn record_os(endpoint: &str, provider: &str) {
    record_os_in(
        &crate::support_dirs::app_data_file(FILE),
        endpoint,
        provider,
    );
}

/// The OS last seen at `endpoint` (`windows`, `linux`, `android`), if any.
pub(super) fn os_of(endpoint: &str) -> Option<String> {
    os_in(&crate::support_dirs::app_data_file(FILE), endpoint)
}

fn record_os_in(file: &Path, endpoint: &str, provider: &str) {
    let Some(family) = HostFamily::of_provider(provider) else {
        return;
    };
    let mut known = read(file);
    let key = key(endpoint);
    if known.get(&key).and_then(Value::as_str) == Some(family.label()) {
        return;
    }
    known.insert(key, Value::from(family.label()));
    let Ok(bytes) = serde_json::to_vec_pretty(&Value::Object(known)) else {
        return;
    };
    // Each process writes its own temporary file; the rename replaces the
    // file at once, so a reader never sees half of it.
    let staged = file.with_extension(format!("json.{}.tmp", std::process::id()));
    if std::fs::write(&staged, bytes).is_err() || std::fs::rename(&staged, file).is_err() {
        let _ = std::fs::remove_file(&staged);
    }
}

fn os_in(file: &Path, endpoint: &str) -> Option<String> {
    read(file)
        .get(&key(endpoint))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn read(file: &Path) -> Map<String, Value> {
    match std::fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    {
        Some(Value::Object(known)) => known,
        _ => Map::new(),
    }
}

/// The device behind `endpoint`: a Direct contact, or a room member by its
/// device id (`connections` and `se exec` name a room differently).
fn key(endpoint: &str) -> String {
    match crate::share::PeerOpenTarget::from_endpoint(endpoint.trim()) {
        Some((crate::share::PeerOpenTarget::Direct { contact_id }, _)) => {
            format!("direct/{contact_id}")
        }
        Some((crate::share::PeerOpenTarget::RoomDevice { device_id, .. }, _)) => {
            format!("device/{device_id}")
        }
        None => endpoint.trim().trim_end_matches('/').to_string(),
    }
}

/// The shell a Share result ran in: the host's own report; for a host
/// before shell reporting (`provider` names it) the shell it always runs
/// once the command exited; `none` for argv; `None` when no shell ran or it
/// is unknown.
pub(super) fn result_shell(
    terminal: &ExecTerminal,
    shell_command: bool,
    provider: Option<&str>,
) -> Option<String> {
    if !shell_command {
        return Some(NO_SHELL.to_string());
    }
    if let Some(shell) = &terminal.shell {
        return Some(shell.clone());
    }
    if terminal.kind != ExecTerminalKind::Exited {
        return None;
    }
    provider
        .and_then(HostFamily::of_provider)
        .map(|family| family.legacy_shell().to_string())
}

#[cfg(test)]
mod tests {
    use super::{os_in, record_os_in, result_shell};
    use crate::share::{ExecId, ExecTerminal, ExecTerminalKind};

    #[test]
    fn exec_shell_task_peer_os_is_kept_per_endpoint() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("exec-peer-os.json");
        assert_eq!(os_in(&file, "share://direct/a"), None);
        record_os_in(&file, "share://direct/a/", "Windows Job Object");
        // A room member is the same device under every room id.
        record_os_in(&file, "share://room/r/d", "systemd transient cgroup");
        record_os_in(&file, "share://direct/b", "unknown provider");
        assert_eq!(os_in(&file, "share://direct/a").as_deref(), Some("windows"));
        assert_eq!(os_in(&file, "share://room/r/d/").as_deref(), Some("linux"));
        assert_eq!(
            os_in(&file, "share://room/other/d").as_deref(),
            Some("linux")
        );
        assert_eq!(os_in(&file, "share://direct/b"), None);
        // A peer that changed its system is named by its last answer.
        record_os_in(&file, "share://direct/a", "android-subreaper");
        assert_eq!(os_in(&file, "share://direct/a").as_deref(), Some("android"));
        std::fs::write(&file, b"not json").unwrap();
        assert_eq!(os_in(&file, "share://direct/a"), None);
    }

    #[test]
    fn exec_shell_task_results_name_the_shell_that_ran() {
        let mut terminal = ExecTerminal {
            exec_id: ExecId::parse("00".repeat(16)).unwrap(),
            kind: ExecTerminalKind::Exited,
            exit_code: Some(0),
            signal: None,
            message: None,
            stdout_bytes: 0,
            stderr_bytes: 0,
            output_truncated: false,
            shell: Some("pwsh 7".into()),
        };
        let windows = Some("Windows Job Object");
        assert_eq!(
            result_shell(&terminal, true, windows).as_deref(),
            Some("pwsh 7")
        );
        assert_eq!(
            result_shell(&terminal, false, windows).as_deref(),
            Some("none")
        );
        // A host before shell reporting ran its own shell: cmd on Windows.
        terminal.shell = None;
        assert_eq!(
            result_shell(&terminal, true, windows).as_deref(),
            Some("cmd")
        );
        let linux = Some("systemd transient cgroup");
        assert_eq!(
            result_shell(&terminal, true, linux).as_deref(),
            Some("login shell")
        );
        assert_eq!(result_shell(&terminal, true, None), None);
        // Nothing ran: no shell is named.
        terminal.kind = ExecTerminalKind::Failed;
        assert_eq!(result_shell(&terminal, true, windows), None);
    }
}
