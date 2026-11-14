use super::inventory::{argument, attach_listener, certificate, service, unit};
use crate::rdp::{
    core::{HostKind, HostProbe, SessionMode, SetupState},
    os::shared::process::{capture, capture_with_env, find_tool},
};
use std::path::Path;

pub(super) fn probe(report: &mut HostProbe) {
    if find_tool(&["krdpserver"]).is_none() {
        return;
    }
    let mut found = service("krdp", HostKind::Krdp, SessionMode::SharedConsole);
    match unit("app-org.kde.krdpserver.service", true, "krdpserver") {
        Err(error) => found.unavailable_reason = Some(error),
        Ok(unit) => {
            // E56: a running KRdp whose command does not bind loopback listens on every
            // interface. se does not change it; the probe reports it as evidence and as
            // one diagnosis line.
            if let Some((evidence, note)) = all_interfaces_notes(unit.active, &unit.command) {
                found.evidence.push(evidence.into());
                if !report.diagnostics.iter().any(|line| line == note) {
                    report.diagnostics.push(note.into());
                }
            }
            let read_config = |key: &str, default: &str| -> Result<String, String> {
                let tool = find_tool(&["kreadconfig6"]).ok_or_else(||
                    "kreadconfig6 is unavailable; effective user/system KDE configuration cannot be proved".to_string())?;
                let help = capture(&tool, &["--help"])?;
                if !help.contains("--include-globals") {
                    return Err("Installed kreadconfig6 cannot include the server's global KDE settings; its effective session mode cannot be proved".into());
                }
                capture(
                    &tool,
                    &[
                        "--include-globals",
                        "--file",
                        "krdpserverrc",
                        "--group",
                        "General",
                        "--key",
                        key,
                        "--default",
                        default,
                    ],
                )
                .map(|value| {
                    value
                        .trim_end_matches(|c| c == '\r' || c == '\n')
                        .to_string()
                })
            };
            let configuration = (|| {
                Ok::<_, String>((
                    read_config("ListenPort", "3389")?,
                    read_config("Certificate", "")?,
                    read_config("ExclusiveMode", "false")?,
                ))
            })();
            match configuration {
                Err(error) => {
                    attach_listener(&mut found, &unit, None);
                    found.unavailable_reason = Some(error);
                }
                Ok((configured_port, configured_cert, exclusive)) => {
                    let port = argument(&unit.command, "--port")
                        .unwrap_or(&configured_port)
                        .parse::<u16>()
                        .ok();
                    let cert = argument(&unit.command, "--certificate").unwrap_or(&configured_cert);
                    attach_listener(&mut found, &unit, port);
                    if port.is_none() {
                        found.unavailable_reason =
                            Some("KRdp effective configured port is invalid".into());
                    }
                    let image = std::fs::read_link(format!("/proc/{}/exe", unit.pid)).ok();
                    let help = image.as_ref().and_then(|path| {
                        capture_with_env(path, &["--help"], &[("QT_QPA_PLATFORM", "offscreen")])
                            .ok()
                    });
                    let supports_modes = help.as_ref().is_some_and(|help| {
                        help.contains("RemoteAccess") && help.contains("AdditionalDisplay")
                    });
                    found.version = image
                        .as_ref()
                        .and_then(|path| {
                            capture_with_env(
                                path,
                                &["--version"],
                                &[("QT_QPA_PLATFORM", "offscreen")],
                            )
                            .ok()
                        })
                        .map(|version| version.trim().chars().take(256).collect());
                    found.session_mode = if let Some(mode) = argument(&unit.command, "--mode") {
                        if !supports_modes {
                            SessionMode::Unknown
                        } else {
                            match mode {
                                "SharedAccess" => SessionMode::SharedConsole,
                                "RemoteAccess" => SessionMode::ExclusiveConsole,
                                "AdditionalDisplay" => SessionMode::AdditionalConsoleDisplay,
                                _ => SessionMode::Unknown,
                            }
                        }
                    } else if matches!(
                        exclusive.to_ascii_lowercase().as_str(),
                        "true" | "on" | "yes" | "1"
                    ) {
                        if supports_modes {
                            SessionMode::ExclusiveConsole
                        } else {
                            SessionMode::Unknown
                        }
                    } else if matches!(
                        exclusive.to_ascii_lowercase().as_str(),
                        "false" | "off" | "no" | "0"
                    ) {
                        SessionMode::SharedConsole
                    } else {
                        SessionMode::Unknown
                    };
                    let virtual_monitor = argument(&unit.command, "--virtual-monitor").is_some();
                    let monitor = argument(&unit.command, "--monitor").is_some();
                    if (virtual_monitor || monitor)
                        && found.session_mode == SessionMode::ExclusiveConsole
                    {
                        found.session_mode = SessionMode::Unknown;
                        found.unavailable_reason=Some("KRdp remote-access mode plus an explicit monitor override changes its greeter/lock behavior; this combination has no verified session-effect adapter".into());
                    } else if virtual_monitor && found.session_mode != SessionMode::Unknown {
                        found.session_mode = SessionMode::AdditionalConsoleDisplay;
                    }
                    if !cert.is_empty() {
                        certificate(&mut found, Path::new(cert));
                    }
                    if found.certificate_sha256.is_some() {
                        found.tls = Some(true);
                    } else {
                        found.evidence.push("KRdp configured certificate is unreadable or absent; no ephemeral fingerprint or default certificate path is invented".into());
                    }
                    found.evidence.push("Native kreadconfig6 resolves the effective user/system KDE configuration; active server argv overrides are applied. KRdp uses the existing Plasma session, not SDDM remote login".into());
                }
            }
        }
    }
    report.services.push(found);
}

/// One `krdpserverrc` `[General]` value through `kreadconfig6`.
fn general(key: &str) -> Result<String, String> {
    let tool = find_tool(&["kreadconfig6"]).ok_or_else(|| "kreadconfig6_missing".to_string())?;
    let args = ["--file", "krdpserverrc", "--group", "General", "--key", key];
    capture(&tool, &args).map(|value| value.trim().to_string())
}

/// Whether KRdp may admit anyone: configured KRdp users or the system-user
/// login (`SystemUserEnabled`). se creates no KRdp users; KRdp keeps their
/// passwords itself (design 3.3).
pub fn has_users(users: &str, system_user: &str) -> bool {
    let system = matches!(
        system_user.to_ascii_lowercase().as_str(),
        "true" | "on" | "yes" | "1"
    );
    system || users.split(',').any(|user| !user.trim().is_empty())
}

/// Plasma session with `krdpserver` (design 3.3): start the user unit when
/// KRdp can admit a user; otherwise say what the person has to add.
pub(super) fn configure() -> SetupState {
    let users = match general("Users") {
        Ok(users) => users,
        Err(error) => return SetupState::Failed(format!("krdp_config: {error}")),
    };
    let system_user = general("SystemUserEnabled").unwrap_or_default();
    if !has_users(&users, &system_user) {
        return SetupState::Impossible("krdp_no_users".into());
    }
    let Some(systemctl) = find_tool(&["systemctl"]) else {
        return SetupState::Failed("systemctl_missing".into());
    };
    // E29: a KRdp that se sets up listens on loopback only.
    if let Err(error) = ensure_loopback(&systemctl) {
        return SetupState::Failed(format!("krdp_address: {error}"));
    }
    let unit = [
        "--user",
        "enable",
        "--now",
        "app-org.kde.krdpserver.service",
    ];
    match super::host_setup::run_tool(&systemctl, &unit, None) {
        Ok(_) => SetupState::Done,
        Err(error) => SetupState::Failed(format!("krdp_unit: {error}")),
    }
}

/// The KRdp user unit that se configures.
const KRDP_UNIT: &str = "app-org.kde.krdpserver.service";
/// E29: a KRdp that se sets up listens on loopback. KRdp (`server/main.cpp`,
/// read 2026-11-08) has no configuration key and no environment variable for
/// its listen address; `--address` defaults to 0.0.0.0 and is a command-line
/// argument only.
const LOOPBACK_ARGS: [&str; 2] = ["--address", "127.0.0.1"];

/// The argv of the unit's `ExecStart` as `systemctl show --property=ExecStart`
/// prints it, for example `{ path=… ; argv[]=… ; ignore_errors=no ; … }`.
pub(super) fn parse_exec_start(show: &str) -> Result<Vec<String>, String> {
    let rest = show
        .split_once("argv[]=")
        .map(|(_, rest)| rest)
        .ok_or_else(|| "KRdp ExecStart has no argv".to_string())?;
    let argument = rest.split(" ; ").next().unwrap_or(rest);
    let argv: Vec<String> = argument
        .trim_end_matches(|c: char| c == '}' || c.is_whitespace())
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if argv.is_empty() {
        return Err("KRdp ExecStart has no argv".into());
    }
    Ok(argv)
}

/// The argv of the unit's effective `ExecStart` from the unit text that
/// `systemctl --user cat` prints: the unit file and its drop-ins in load order.
/// An empty `ExecStart=` clears the earlier list, as systemd reads it (systemd.service).
/// Only one plain command is accepted: an absolute path and arguments separated by
/// whitespace, without quotes, backslashes, `$`, `%`, `;` or a command prefix. Its
/// whitespace is the argument boundary that systemd's quoting rules give (O7); any
/// other text is refused, so the show output's flattened words are never used alone.
pub(super) fn exec_start_from_unit_text(text: &str) -> Result<Vec<String>, String> {
    let mut commands: Vec<&str> = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        if key.trim() != "ExecStart" {
            continue;
        }
        let value = value.trim();
        if value.is_empty() {
            commands.clear();
        } else {
            commands.push(value);
        }
    }
    let command = match commands.as_slice() {
        [single] => *single,
        _ => return Err("KRdp unit has no single ExecStart command; nothing is changed".into()),
    };
    if !command.starts_with('/') {
        return Err(
            "KRdp ExecStart has a command prefix or no absolute path; nothing is changed".into(),
        );
    }
    if command
        .chars()
        .any(|c| matches!(c, '"' | '\'' | '\\' | '$' | '%' | ';'))
    {
        return Err(
            "KRdp ExecStart uses quoting, escapes or specifiers; nothing is changed".into(),
        );
    }
    Ok(command.split_whitespace().map(str::to_string).collect())
}

/// The argv with exactly one `--address 127.0.0.1`: every earlier address
/// argument (separate or `--address=` form) is removed, the loopback pair is
/// appended, every other argument keeps its order.
pub(super) fn loopback_argv(argv: &[String]) -> Vec<String> {
    let mut kept = Vec::with_capacity(argv.len() + LOOPBACK_ARGS.len());
    let mut skip_value = false;
    for arg in argv {
        if skip_value {
            skip_value = false;
        } else if arg.as_str() == "--address" {
            skip_value = true;
        } else if !arg.starts_with("--address=") {
            kept.push(arg.clone());
        }
    }
    kept.extend(LOOPBACK_ARGS.iter().map(|part| part.to_string()));
    kept
}

/// E56: evidence for a running KRdp that does not bind loopback. se does not
/// change such a unit; the probe reports it.
const KRDP_NETWORK_REACH_EVIDENCE: &str =
    "KRdp lauscht auf allen Schnittstellen (nicht von se auf Loopback gestellt)";
/// E56: the same fact as one diagnosis line of the probe.
const KRDP_NETWORK_REACH_NOTE: &str = "KRdp ist aus dem lokalen Netz erreichbar, solange der Dienst läuft: er lauscht auf allen Schnittstellen, weil er nicht auf 127.0.0.1 gestellt ist. Eine Firewall kann das begrenzen. se stellt den Dienst nicht um; eine Umstellung ist eine Entscheidung der Person, die ihn eingerichtet hat.";

/// Whether the command binds loopback: every `--address` argument (separate or
/// `=` form) names a loopback address (127.0.0.0/8 or ::1, as 127.0.0.1 that se
/// sets), and there is at least one. Without an address argument KRdp listens on
/// 0.0.0.0. Every argument is checked, so a later non-loopback address is not
/// hidden by an earlier loopback one.
fn loopback_bound(command: &[String]) -> bool {
    let mut addresses: Vec<Option<&str>> = Vec::new();
    let mut rest = command.iter();
    while let Some(arg) = rest.next() {
        if arg == "--address" {
            addresses.push(rest.next().map(String::as_str));
        } else if let Some(value) = arg.strip_prefix("--address=") {
            addresses.push(Some(value));
        }
    }
    let loopback = |value: &&str| {
        value
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
    };
    !addresses.is_empty()
        && addresses
            .iter()
            .all(|value| value.as_ref().is_some_and(loopback))
}

/// The evidence and the diagnosis line for a KRdp unit that runs without
/// loopback binding. `None` when it is not running, when its command is not
/// known (no claim without a proof), or when the command binds loopback.
fn all_interfaces_notes(running: bool, command: &[String]) -> Option<(&'static str, &'static str)> {
    (running && !command.is_empty() && !loopback_bound(command))
        .then_some((KRDP_NETWORK_REACH_EVIDENCE, KRDP_NETWORK_REACH_NOTE))
}

/// A token that systemd reads back unchanged: no quoting, no separator, no
/// specifier. Any other argument is refused instead of being rewritten.
fn plain_token(arg: &str) -> Result<&str, String> {
    let special = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '$' | '%' | ';');
    if arg.is_empty() || arg.chars().any(special) {
        Err("KRdp argument cannot be written to the user unit unchanged".into())
    } else {
        Ok(arg)
    }
}

/// The drop-in text: the empty `ExecStart=` clears the unit's command, the
/// following line sets the complete argv.
pub(super) fn dropin_text(argv: &[String]) -> Result<String, String> {
    let tokens = argv
        .iter()
        .map(|arg| plain_token(arg))
        .collect::<Result<Vec<&str>, String>>()?;
    Ok(format!(
        "[Service]\nExecStart=\nExecStart={}\n",
        tokens.join(" ")
    ))
}

/// `$XDG_CONFIG_HOME/systemd/user/<unit>.d/se-loopback.conf`, the user's own
/// unit directory (se configures KRdp in the user session, not as root).
fn dropin_path() -> Result<std::path::PathBuf, String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })
        .ok_or_else(|| "no user configuration directory".to_string())?;
    Ok(config
        .join("systemd")
        .join("user")
        .join(format!("{KRDP_UNIT}.d"))
        .join("se-loopback.conf"))
}

/// E29: writes the loopback drop-in when the configured argv differs, then
/// reloads the user manager and restarts KRdp so that it listens on loopback.
/// An argv that already matches changes nothing.
fn ensure_loopback(systemctl: &Path) -> Result<(), String> {
    let show = capture(
        systemctl,
        &[
            "--user",
            "show",
            "--no-pager",
            "--property=ExecStart",
            KRDP_UNIT,
        ],
    )?;
    let shown = parse_exec_start(&show)?;
    // O7: the unit text keeps the argument boundaries that the show output
    // flattens; both must name the same command, otherwise nothing is changed.
    let unit_text = capture(systemctl, &["--user", "cat", "--no-pager", KRDP_UNIT])?;
    let argv = exec_start_from_unit_text(&unit_text)?;
    if argv != shown {
        return Err(
            "KRdp: Unit-Text und systemd-Ausgabe nennen verschiedene Befehle; nichts geändert"
                .into(),
        );
    }
    let wanted = loopback_argv(&argv);
    if wanted == argv {
        return Ok(());
    }
    let text = dropin_text(&wanted)?;
    let path = dropin_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| format!("krdp_dropin_dir: {error}"))?;
    }
    crate::support_dirs::write_private_atomic(&path, text.as_bytes())
        .map_err(|error| format!("krdp_dropin: {error}"))?;
    super::host_setup::run_tool(systemctl, &["--user", "daemon-reload"], None)?;
    super::host_setup::run_tool(systemctl, &["--user", "restart", KRDP_UNIT], None)?;
    Ok(())
}

#[cfg(test)]
#[path = "rdp_netz_krdp_task_tests.rs"]
mod rdp_netz_krdp_task_tests;
