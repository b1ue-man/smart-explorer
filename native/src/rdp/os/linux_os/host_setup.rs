//! Linux RDP self-setup (design 3.3). Decision tree: an existing GNOME or
//! KRdp backend of the session, else xrdp on loopback only, else install the
//! package that fits the session. Root steps run directly in a root service
//! and otherwise through `pkexec <se> rdp host-setup --system <step>` (the
//! polkit dialog is the OS approval, like UAC on Windows).
use crate::rdp::core::{HostKind, HostProbe, HostService, SetupState};
use crate::rdp::os::shared::process::find_tool;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// The polkit path of the root steps (`pkexec <se> rdp host-setup …`).
#[path = "host_setup_pkexec.rs"]
mod pkexec;
#[cfg(test)]
pub(crate) use pkexec::pkexec_outcome;
use pkexec::via_pkexec;

// Pure decisions of the setup adapters for the task suite.
#[cfg(test)]
pub(crate) use super::gnome::{credential_input, managed_username};
#[cfg(test)]
pub(crate) use super::krdp::has_users;
#[cfg(test)]
pub(crate) use super::xrdp::{loopback_config, parse_port};

/// se installed xrdp itself; only then a network-wide xrdp becomes loopback.
const XRDP_MARKER: &str = "/var/lib/smart-explorer/rdp-xrdp-installed";
/// Design 7 keeps the tool guard of `process.rs` (hanging `grdctl`,
/// `kreadconfig6`, `systemctl --user`); package managers and polkit wait
/// for the person or the network without a fixed limit.
const TOOL_GUARD: Duration = Duration::from_secs(10);
const OUTPUT_LIMIT: u64 = 128 * 1024;

/// The fixed root steps; `pkexec` receives one of these names, never free text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SystemStep {
    InstallXrdp,
    ConfigureXrdp,
    InstallGnomeRd,
    InstallKrdp,
}
impl SystemStep {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "install-xrdp" => Self::InstallXrdp,
            "configure-xrdp" => Self::ConfigureXrdp,
            "install-gnome-rd" => Self::InstallGnomeRd,
            "install-krdp" => Self::InstallKrdp,
            _ => return None,
        })
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::InstallXrdp => "install-xrdp",
            Self::ConfigureXrdp => "configure-xrdp",
            Self::InstallGnomeRd => "install-gnome-rd",
            Self::InstallKrdp => "install-krdp",
        }
    }
    fn packages(self) -> &'static [&'static str] {
        match self {
            Self::InstallXrdp => &["xrdp", "xorgxrdp"],
            Self::ConfigureXrdp => &[],
            Self::InstallGnomeRd => &["gnome-remote-desktop"],
            Self::InstallKrdp => &["krdp"],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PackageManager {
    Apt,
    Dnf,
}

/// `apt-get` for Debian/Ubuntu, `dnf` for Fedora/RHEL, read from the `ID`
/// and `ID_LIKE` lines of `/etc/os-release`.
pub(crate) fn package_manager_from(os_release: &str) -> Option<PackageManager> {
    let mut ids = Vec::new();
    for line in os_release.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "ID" || key.trim() == "ID_LIKE" {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            ids.extend(value.split_whitespace().map(str::to_ascii_lowercase));
        }
    }
    let has = |name: &str| ids.iter().any(|id| id == name);
    if has("debian") || has("ubuntu") {
        Some(PackageManager::Apt)
    } else if has("fedora") || has("rhel") {
        Some(PackageManager::Dnf)
    } else {
        None
    }
}

fn package_manager() -> Result<PackageManager, SetupState> {
    let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    package_manager_from(&text).ok_or_else(|| {
        SetupState::Impossible("package_manager_unsupported (xrdp, xorgxrdp)".into())
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Desktop {
    Gnome,
    Plasma,
    Other,
}

/// The session's desktop from `XDG_CURRENT_DESKTOP` (e.g. `ubuntu:GNOME`).
pub(crate) fn desktop_from(current: &str) -> Desktop {
    let parts: Vec<_> = current.split(':').map(str::to_ascii_uppercase).collect();
    if parts.iter().any(|part| part == "GNOME") {
        Desktop::Gnome
    } else if parts.iter().any(|part| part == "KDE") {
        Desktop::Plasma
    } else {
        Desktop::Other
    }
}

fn is_root() -> bool {
    // SAFETY: geteuid takes no pointer and has no side effects.
    unsafe { libc::geteuid() == 0 }
}

fn has_display() -> bool {
    ["DISPLAY", "WAYLAND_DISPLAY"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

fn last_line(text: &[u8]) -> String {
    let text = String::from_utf8_lossy(text);
    let line = text.lines().rev().find(|line| !line.trim().is_empty());
    line.unwrap_or("").trim().chars().take(512).collect()
}

/// Reads one child pipe to its end (bounded) on its own thread.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(pipe) = pipe {
            let _ = pipe.take(OUTPUT_LIMIT).read_to_end(&mut bytes);
        }
        bytes
    })
}

/// A setup tool with optional stdin, under the tool guard. The output is
/// returned; a failure names the tool, exit code and its last stderr line.
pub(super) fn run_tool(
    program: &Path,
    args: &[&str],
    input: Option<&[u8]>,
) -> Result<String, String> {
    let name = program.display().to_string();
    let stdin = if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{name}: {error}"))?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    if let (Some(bytes), Some(mut pipe)) = (input, child.stdin.take()) {
        let written = pipe.write_all(bytes);
        drop(pipe);
        if let Err(error) = written {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{name} stdin: {error}"));
        }
    }
    let deadline = Instant::now() + TOOL_GUARD;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{name} exceeded the ten-second tool guard"));
            }
            Err(error) => return Err(format!("{name} wait: {error}")),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        let code = status.code().unwrap_or(-1);
        return Err(format!("{name} exit {code}: {}", last_line(&stderr)));
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

/// A command without a time limit (package managers, `systemctl` system
/// jobs, `pkexec`): it waits for the network, the package lock or a person.
fn run_unbounded(program: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<Output, String> {
    Command::new(program)
        .args(args)
        .envs(env.iter().copied())
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("{}: {error}", program.display()))
}

fn package_failure(output: &Output) -> SetupState {
    let code = output.status.code().unwrap_or(-1);
    SetupState::Failed(format!("package:{code}:{}", last_line(&output.stderr)))
}

/// apt messages of package lists that do not know the package yet.
const STALE_LISTS: &[&str] = &[
    "Unable to locate package",
    "has no installation candidate",
    "Failed to fetch",
];

/// Package lists that do not know the package yet: refresh once, then retry.
fn stale_lists(stderr: &[u8]) -> bool {
    let text = String::from_utf8_lossy(stderr);
    STALE_LISTS.iter().any(|needle| text.contains(needle))
}

fn install(manager: PackageManager, packages: &[&str]) -> Result<(), SetupState> {
    let missing = |name: &str| SetupState::Failed(format!("{name}_missing"));
    match manager {
        PackageManager::Apt => {
            let apt = find_tool(&["apt-get"]).ok_or_else(|| missing("apt-get"))?;
            let env = [("DEBIAN_FRONTEND", "noninteractive")];
            // Waits for a held dpkg lock instead of failing on it.
            let mut args = vec!["-o", "DPkg::Lock::Timeout=-1", "install", "-y"];
            args.push("--no-install-recommends");
            args.extend_from_slice(packages);
            let mut output = run_unbounded(&apt, &args, &env).map_err(SetupState::Failed)?;
            if !output.status.success() && stale_lists(&output.stderr) {
                let update = ["-o", "DPkg::Lock::Timeout=-1", "update"];
                run_unbounded(&apt, &update, &env).map_err(SetupState::Failed)?;
                output = run_unbounded(&apt, &args, &env).map_err(SetupState::Failed)?;
            }
            if output.status.success() {
                Ok(())
            } else {
                Err(package_failure(&output))
            }
        }
        PackageManager::Dnf => {
            let dnf = find_tool(&["dnf"]).ok_or_else(|| missing("dnf"))?;
            let mut args = vec!["install", "-y"];
            args.extend_from_slice(packages);
            let output = run_unbounded(&dnf, &args, &[]).map_err(SetupState::Failed)?;
            if output.status.success() {
                Ok(())
            } else {
                Err(package_failure(&output))
            }
        }
    }
}

fn systemctl(args: &[&str]) -> Result<(), SetupState> {
    let missing = || SetupState::Failed("systemctl_missing".into());
    let tool = find_tool(&["systemctl"]).ok_or_else(missing)?;
    let output = run_unbounded(&tool, args, &[]).map_err(SetupState::Failed)?;
    if output.status.success() {
        return Ok(());
    }
    let detail = last_line(&output.stderr);
    Err(SetupState::Failed(format!("xrdp_service: {detail}")))
}

/// xrdp on loopback only (design 3.3), then enabled and (re)started.
fn configure_xrdp() -> Result<(), SetupState> {
    let path = super::xrdp::config_path();
    let text = std::fs::read_to_string(&path)
        .map_err(|e| SetupState::Failed(format!("xrdp_config_read: {e}")))?;
    let se_installed = Path::new(XRDP_MARKER).is_file();
    let (rewritten, changed) = match super::xrdp::loopback_config(&text, se_installed) {
        Ok(result) => result,
        Err(reason) if reason == "xrdp_network_wide_not_se_installed" => {
            return Err(SetupState::Impossible(reason));
        }
        Err(reason) => return Err(SetupState::Failed(reason)),
    };
    if changed {
        replace_file(&path, rewritten.as_bytes())
            .map_err(|e| SetupState::Failed(format!("xrdp_config_write: {e}")))?;
    }
    systemctl(&["enable", "--now", "xrdp"])?;
    let port = super::xrdp::configured_port(&path).map_err(SetupState::Failed)?;
    if changed || !listening(port)? {
        systemctl(&["restart", "xrdp"])?;
    }
    wait_listening(port)
}

fn listening(port: u16) -> Result<bool, SetupState> {
    super::xrdp::loopback_listener(port).map_err(SetupState::Failed)
}

/// xrdp forks before it binds: "done" only once its loopback listener
/// exists. A freshly started xrdp either binds or ends itself (bind
/// failure), so the wait has no fixed limit (user rule r7).
fn wait_listening(port: u16) -> Result<(), SetupState> {
    let missing = || SetupState::Failed("systemctl_missing".into());
    let tool = find_tool(&["systemctl"]).ok_or_else(missing)?;
    loop {
        if listening(port)? {
            return Ok(());
        }
        let output =
            run_unbounded(&tool, &["is-active", "xrdp"], &[]).map_err(SetupState::Failed)?;
        let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !matches!(state.as_str(), "active" | "activating" | "reloading") {
            return Err(SetupState::Failed(format!(
                "xrdp_not_listening: {state}: {}",
                journal_tail()
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The last line xrdp wrote to the journal (the reason it stopped).
fn journal_tail() -> String {
    let Some(journal) = find_tool(&["journalctl"]) else {
        return String::new();
    };
    let args = ["-u", "xrdp", "-n", "20", "--no-pager", "-o", "cat"];
    run_unbounded(&journal, &args, &[])
        .map(|output| last_line(&output.stdout))
        .unwrap_or_default()
}

/// Same directory, same permissions, atomic rename.
fn replace_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let permissions = std::fs::metadata(path)?.permissions();
    let staged = path.with_extension("ini.se-staged");
    let mut file = std::fs::File::create(&staged)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::set_permissions(&staged, permissions)?;
    std::fs::rename(&staged, path)
}

/// One root step, in a root process.
fn system_step(step: SystemStep) -> SetupState {
    if !is_root() {
        return SetupState::Failed("system_step_needs_root".into());
    }
    let result = (|| {
        if step != SystemStep::ConfigureXrdp {
            let fresh_xrdp = find_tool(&["xrdp"]).is_none();
            install(package_manager()?, step.packages())?;
            if step == SystemStep::InstallXrdp && fresh_xrdp {
                let marker = Path::new(XRDP_MARKER);
                if let Some(parent) = marker.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| SetupState::Failed(format!("xrdp_marker: {e}")))?;
                }
                std::fs::write(marker, b"")
                    .map_err(|e| SetupState::Failed(format!("xrdp_marker: {e}")))?;
            }
        }
        if matches!(step, SystemStep::InstallXrdp | SystemStep::ConfigureXrdp) {
            configure_xrdp()?;
        }
        Ok::<(), SetupState>(())
    })();
    result.err().unwrap_or(SetupState::Done)
}

fn system(step: SystemStep) -> SetupState {
    if is_root() {
        system_step(step)
    } else {
        via_pkexec(step)
    }
}

fn gnome(device_id: Option<&str>) -> SetupState {
    match device_id {
        Some(device_id) => super::gnome::configure(device_id),
        None => SetupState::Failed("device_identity_unknown".into()),
    }
}

/// Linux hosts RDP; distribution limits show inside `ensure`.
pub(crate) fn impossible() -> Option<SetupState> {
    None
}

/// Design 3.3 decision tree; the caller has already found no startable
/// service. A root service cannot configure a user's GNOME or KRdp session
/// and therefore takes the xrdp branch.
pub(crate) fn ensure(_probe: &HostProbe, device_id: Option<&str>) -> SetupState {
    let desktop = if is_root() {
        Desktop::Other
    } else {
        desktop_from(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default())
    };
    match desktop {
        Desktop::Gnome if find_tool(&["grdctl"]).is_some() => return gnome(device_id),
        Desktop::Plasma if find_tool(&["krdpserver"]).is_some() => return super::krdp::configure(),
        _ => {}
    }
    if find_tool(&["xrdp"]).is_some() {
        return system(SystemStep::ConfigureXrdp);
    }
    // Before any prompt: an unknown distribution cannot install anything.
    if let Err(state) = package_manager() {
        return state;
    }
    match desktop {
        Desktop::Gnome => match system(SystemStep::InstallGnomeRd) {
            SetupState::Done => gnome(device_id),
            other => other,
        },
        Desktop::Plasma => match system(SystemStep::InstallKrdp) {
            SetupState::Done => super::krdp::configure(),
            other => other,
        },
        Desktop::Other => system(SystemStep::InstallXrdp),
    }
}

/// Only the GNOME user backend that se set up has se-managed credentials.
pub(crate) fn issue_credential(
    service: &HostService,
    device_id: &str,
) -> Result<(String, String), String> {
    if service.kind != HostKind::Gnome || service.id != "gnome-console" {
        return Err(format!(
            "RDP-Dienst {} meldet mit dem Systemkonto an",
            service.id
        ));
    }
    super::gnome::issue_credential(device_id)
}

/// Rotation after a revocation and withdrawal when no device steers any
/// more concern only the GNOME sharing se manages (xrdp set up by se listens
/// on loopback only and stays).
pub(crate) use super::gnome::{
    rotate_credential as rotate_credentials, withdraw as withdraw_managed,
};

/// What decides whether this process can show a polkit prompt: root, and
/// the graphical session of its environment (fixed for the process).
pub(crate) fn session_facts() -> String {
    let var = |name: &str| std::env::var(name).unwrap_or_default();
    format!(
        "root={}|display={}|wayland={}|desktop={}",
        is_root(),
        var("DISPLAY"),
        var("WAYLAND_DISPLAY"),
        var("XDG_CURRENT_DESKTOP")
    )
}

/// `se rdp host-setup --system <step>`: `state=<state>` and exit code 0
/// when done, 1 otherwise. Linux steps take no user SID.
pub(crate) fn run_system_step(step: &str, user_sid: Option<&str>) -> (String, i32) {
    let state = match (SystemStep::parse(step), user_sid) {
        (Some(step), None) => system_step(step),
        _ => SetupState::Failed("unknown_system_step".into()),
    };
    let code = i32::from(!matches!(state, SetupState::Done | SetupState::NotNeeded));
    (format!("state={state}"), code)
}
