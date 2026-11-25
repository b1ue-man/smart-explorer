//! The privileged pre-start step of a hardened systemd service (user orders
//! 2026-11-25: „update, installation, NUTZUNG richten das ein. wenn es fehlt
//! wird es eingerichtet“, „SE MUSS ES MACHEN! AUTOMATISCH! ELEVATION FEHLT?
//! ERFRAGEN!“). A service with `User=`, `NoNewPrivileges=true` and an empty
//! `CapabilityBoundingSet=` is neither root nor able to use sudo, so its start
//! cannot admit the ports (`firewall_ports`). systemd runs an
//! `ExecStartPre=+…` command as root regardless of `User=` and the sandbox:
//! `--install-firewall-hook` writes `se-share-server <ExecStart arguments>
//! --admit-firewall` as such a step into a drop-in of the unit, so every start,
//! also the restart after an update, admits the ports first.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::PathBuf;

use crate::config::Options;
use crate::firewall_ports::{self, Listener};

/// Only admit the ports of the given settings, then exit 0.
pub(crate) const ADMIT_FLAG: &str = "--admit-firewall";
/// Write the pre-start step into the unit's drop-in.
pub(crate) const INSTALL_FLAG: &str = "--install-firewall-hook";
pub(crate) const DEFAULT_UNIT: &str = "se-share-server.service";
pub(crate) const DROP_IN: &str = "se-firewall.conf";
const UNIT_DIR: &str = "/etc/systemd/system";
/// The firewall services whose rules stand before the step adds the server's
/// ports (nftables and iptables rules of the server are not persistent).
const FIREWALL_UNITS: &str = "ufw.service firewalld.service nftables.service \
    netfilter-persistent.service iptables.service ip6tables.service";

/// `UNIT` or `UNIT.service`, only from the characters of systemd unit names.
pub(crate) fn unit_name(value: &str) -> Result<String, String> {
    let value = value.trim();
    let valid = !value.is_empty()
        && !value.starts_with('.')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ":-_.\\@".contains(c));
    if !valid {
        return Err(format!("--unit {value}: not a systemd service name"));
    }
    Ok(if value.ends_with(".service") {
        value.to_string()
    } else {
        format!("{value}.service")
    })
}

pub(crate) fn drop_in_dir(unit: &str) -> PathBuf {
    PathBuf::from(UNIT_DIR).join(format!("{unit}.d"))
}

pub(crate) fn drop_in_path(unit: &str) -> PathBuf {
    drop_in_dir(unit).join(DROP_IN)
}

/// The program and arguments of a service.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExecStart {
    pub(crate) path: String,
    pub(crate) args: Vec<String>,
}

/// Reads `systemctl show UNIT -p ExecStart --value`:
/// `{ path=/opt/…/se-share-server ; argv[]=/opt/…/se-share-server ARGS ;
/// ignore_errors=no ; … }` (`flags=` in the ExecStartEx form). systemd joins
/// argv with single spaces, so the arguments are split at whitespace; argv[0]
/// is dropped and an `--admit-firewall` left out.
pub(crate) fn parse_exec_start(value: &str) -> Result<ExecStart, String> {
    let entries: Vec<&str> = value
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("{ path="))
        .collect();
    let entry =
        match entries.as_slice() {
            [entry] => *entry,
            [] => return Err(
                "kein ExecStart (Dienst nicht installiert oder --unit nennt einen anderen Dienst)"
                    .into(),
            ),
            _ => return Err(format!("{} ExecStart-Einträge statt einem", entries.len())),
        };
    let (path, rest) = entry["{ path=".len()..]
        .split_once(" ; argv[]=")
        .ok_or("ExecStart ohne argv[]")?;
    let end = [" ; ignore_errors=", " ; flags="]
        .iter()
        .filter_map(|marker| rest.find(marker))
        .min()
        .ok_or("ExecStart ohne Ende von argv[]")?;
    let path = path.trim();
    let name = path.rsplit('/').next().unwrap_or(path);
    if !name.starts_with("se-share-server") {
        return Err(format!("ExecStart startet {path}, nicht se-share-server"));
    }
    let args = rest[..end]
        .split_whitespace()
        .skip(1)
        .filter(|arg| *arg != ADMIT_FLAG)
        .map(str::to_string)
        .collect();
    Ok(ExecStart {
        path: path.to_string(),
        args,
    })
}

/// One word of a systemd command line: `%` doubled (specifiers), quoted with
/// C escapes when it holds whitespace, quotes or backslashes, is empty or a
/// lone `;`. `$` stays, so systemd substitutes variables as in ExecStart.
pub(crate) fn exec_word(word: &str) -> String {
    let word = word.replace('%', "%%");
    let plain = !word.is_empty()
        && word != ";"
        && word
            .chars()
            .all(|c| !c.is_whitespace() && !c.is_control() && !"\"'\\".contains(c));
    if plain {
        return word;
    }
    let mut quoted = String::from("\"");
    for c in word.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

/// `ExecStartPre=+-EXE ARGS --admit-firewall`: `+` runs it as root without
/// `User=` and the sandbox, `-` lets a failure never stop the start.
pub(crate) fn pre_start_line(exe: &str, args: &[String]) -> String {
    let words: Vec<String> = std::iter::once(exe)
        .chain(
            args.iter()
                .map(String::as_str)
                .filter(|arg| *arg != ADMIT_FLAG),
        )
        .chain([ADMIT_FLAG])
        .map(exec_word)
        .collect();
    format!("ExecStartPre=+-{}", words.join(" "))
}

pub(crate) fn drop_in_text(exe: &str, args: &[String]) -> String {
    format!(
        "# Written by se-share-server {INSTALL_FLAG}, rewritten by it when ExecStart changes.\n\
         # \"+\": systemd runs the step as root, without User= and the sandbox of the service;\n\
         # \"-\": a failure never stops the start. The step admits the server's listening\n\
         # ports in the host firewall and exits 0.\n\
         [Unit]\n\
         After={FIREWALL_UNITS}\n\n\
         [Service]\n\
         {}\n",
        pre_start_line(exe, args)
    )
}

/// The drop-in as the running server sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HookState {
    Missing,
    /// Present with other arguments than this start.
    Stale,
    /// Present with this start's program and arguments; with a `$` variable
    /// in its line the start cannot compare and trusts it.
    Current,
}

pub(crate) fn hook_state(file: Option<&str>, exe: &str, args: &[String]) -> HookState {
    let Some(text) = file else {
        return HookState::Missing;
    };
    let wanted = pre_start_line(exe, args);
    match text
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("ExecStartPre="))
    {
        Some(line) if line == wanted || line.contains('$') => HookState::Current,
        _ => HookState::Stale,
    }
}

/// The system service of `/proc/self/cgroup` (`0::/system.slice/X.service`,
/// cgroup v1 `N:name=systemd:/…`); none in a user manager, a scope or a
/// container root.
pub(crate) fn unit_from_cgroup(text: &str) -> Option<String> {
    let path = text.lines().find_map(|line| {
        let mut parts = line.splitn(3, ':');
        let (id, controllers, path) = (parts.next()?, parts.next()?, parts.next()?);
        ((id == "0" && controllers.is_empty()) || controllers == "name=systemd").then_some(path)
    })?;
    if path.split('/').any(|part| part.starts_with("user@")) {
        return None;
    }
    path.split('/')
        .rev()
        .find(|part| part.ends_with(".service"))
        .map(str::to_string)
}

/// A word for a POSIX shell: plain when safe, else single-quoted.
pub(crate) fn shell_word(word: &str) -> String {
    let safe = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@+,%".contains(c));
    if safe {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// `sudo EXE --install-firewall-hook [--unit UNIT]`.
pub(crate) fn install_command(exe: &str, unit: &str) -> String {
    let mut words = vec![
        "sudo".to_string(),
        shell_word(exe),
        INSTALL_FLAG.to_string(),
    ];
    if unit != DEFAULT_UNIT {
        words.extend(["--unit".to_string(), shell_word(unit)]);
    }
    words.join(" ")
}

/// The line a start without rights logs when the step is missing or stale.
pub(crate) fn hint(state: HookState, unit: &str, command: &str) -> Option<String> {
    match state {
        HookState::Current => None,
        HookState::Missing => Some(format!(
            "{unit} läuft ohne Root-Rechte und ohne Vorstart-Schritt; einmal als root einrichten, \
             danach gibt systemd die Ports vor jedem Start frei: {command}"
        )),
        HookState::Stale => Some(format!(
            "der Vorstart-Schritt in {} nutzt andere Argumente als dieser Start von {unit}; \
             neu einrichten: {command}",
            drop_in_path(unit).display()
        )),
    }
}

/// The NUL-separated fields of `/proc/PID/cmdline` or `/proc/PID/environ`.
pub(crate) fn nul_fields(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .collect()
}

/// `--admit-firewall`: admits the ports of these settings and exits 0, also
/// when admission or the settings fail (the start reports a settings error).
pub(crate) fn admit_and_exit(options: Result<Options, String>) -> ! {
    let ports = options.and_then(|options| {
        let relay = crate::relay::bind_address(&options.bind.to_string())
            .map_err(|error| error.to_string())?;
        Ok(firewall_ports::server_ports(
            options.bind,
            relay,
            options.cert.is_some(),
        ))
    });
    match ports {
        Ok(ports) if ports.is_empty() => eprintln!(
            "se-share-server: firewall: kein Port von außen erreichbar (alle auf Loopback); nichts freizugeben"
        ),
        Ok(ports) => {
            firewall_ports::admit(&ports);
        }
        Err(error) => {
            eprintln!("se-share-server: firewall: {error}; nichts freigegeben")
        }
    }
    std::process::exit(0)
}

/// The start: admits the ports itself, or leaves them to a current pre-start
/// step, and names the command that installs the step when rights are missing.
pub(crate) fn admit_at_start(ports: &[Listener]) {
    if !ports.is_empty() {
        platform::admit_at_start(ports);
    }
}

/// `--install-firewall-hook`: prints the result and exits 0, or 1 on failure.
pub(crate) fn install_and_exit(unit: &str) -> ! {
    let code = match platform::install(unit) {
        Ok(Some(line)) => {
            println!("se-share-server: firewall: {line}");
            0
        }
        // The elevated run printed its own result.
        Ok(None) => 0,
        Err(error) => {
            eprintln!("se-share-server: {INSTALL_FLAG}: {error}");
            1
        }
    };
    std::process::exit(code)
}

#[cfg(target_os = "linux")]
#[path = "firewall_hook_linux.rs"]
mod platform;

#[cfg(not(target_os = "linux"))]
mod platform {
    pub(super) fn admit_at_start(ports: &[super::Listener]) {
        super::firewall_ports::admit(ports);
    }

    pub(super) fn install(_unit: &str) -> Result<Option<String>, String> {
        Err("der Vorstart-Schritt gilt für einen systemd-Dienst unter Linux".into())
    }
}

#[cfg(test)]
#[path = "firewall_hook_tests.rs"]
mod tests;
