//! The Linux part of `firewall_hook`: the service the server runs in, the
//! drop-in, the reload of systemd and the elevation through sudo.
use super::{
    drop_in_dir, drop_in_path, drop_in_text, hint, hook_state, install_command, nul_fields,
    parse_exec_start, unit_from_cgroup, HookState, ADMIT_FLAG, DROP_IN, INSTALL_FLAG,
};
use crate::firewall_ports::{
    self, last_line, program, root, run, stdout, Admission, Listener, SYSTEMCTL,
};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| arg.to_string()).collect()
}

fn own_unit() -> Option<String> {
    unit_from_cgroup(&std::fs::read_to_string("/proc/self/cgroup").ok()?)
}

pub(super) fn admit_at_start(ports: &[Listener]) {
    // Root admits directly; outside a system service sudo may still ask.
    let unit = if root() { None } else { own_unit() };
    let Some(unit) = unit else {
        firewall_ports::admit(ports);
        return;
    };
    let mut argv = std::env::args();
    let exe = argv.next().unwrap_or_default();
    let args: Vec<String> = argv.collect();
    let path = drop_in_path(&unit);
    let file = std::fs::read_to_string(&path).ok();
    let state = hook_state(file.as_deref(), &exe, &args);
    if state == HookState::Current {
        eprintln!(
            "se-share-server: firewall: zuständig ist der Vorstart-Schritt, den systemd vor diesem Start als root ausführt ({}); sein Ergebnis steht in der firewall-Zeile davor",
            path.display()
        );
        return;
    }
    if firewall_ports::admit(ports) == Admission::NoRights {
        let binary = std::env::current_exe()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or(exe);
        if let Some(line) = hint(state, &unit, &install_command(&binary, &unit)) {
            eprintln!("se-share-server: firewall: {line}");
        }
    }
}

/// Writes `text` as `DIR/se-firewall.conf` (0644, `DIR` 0755 when created)
/// through a temporary file; `false` when it already held `text`.
pub(super) fn write_drop_in(dir: &Path, text: &str) -> std::io::Result<bool> {
    let path = dir.join(DROP_IN);
    let mode = |bits: u32| std::fs::Permissions::from_mode(bits);
    if std::fs::read_to_string(&path).is_ok_and(|current| current == text) {
        // The running server reads it to compare its own arguments.
        std::fs::set_permissions(&path, mode(0o644))?;
        return Ok(false);
    }
    if !dir.is_dir() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, mode(0o755))?;
    }
    // systemd reads only `*.conf` of a drop-in directory.
    let temporary = dir.join(format!(".{DROP_IN}.tmp"));
    std::fs::write(&temporary, text)?;
    std::fs::set_permissions(&temporary, mode(0o644))?;
    std::fs::rename(&temporary, &path)?;
    Ok(true)
}

/// Root writes the step from the unit's ExecStart; any other account asks
/// through sudo (in a terminal; `sudo -n` without one).
pub(super) fn install(unit: &str) -> Result<Option<String>, String> {
    if !root() {
        return elevate(unit).map(|()| None);
    }
    let systemctl = program(&SYSTEMCTL)
        .ok_or("systemctl fehlt; der Vorstart-Schritt gilt für einen systemd-Dienst")?;
    let show = |property: &str| -> Result<String, String> {
        let output = run(&words(&[
            systemctl, "show", unit, "-p", property, "--value",
        ]))
        .map_err(|error| format!("systemctl show: {error}"))?;
        if output.status.success() {
            Ok(stdout(&output))
        } else {
            Err(format!("systemctl show {unit}: {}", last_line(&output)))
        }
    };
    let exec = parse_exec_start(&show("ExecStart")?).map_err(|error| format!("{unit}: {error}"))?;
    let dir = drop_in_dir(unit);
    let path = dir.join(DROP_IN);
    let text = drop_in_text(&exec.path, &exec.args);
    let changed =
        write_drop_in(&dir, &text).map_err(|error| format!("{}: {error}", path.display()))?;
    let loaded = |show: &dyn Fn(&str) -> Result<String, String>| -> Result<bool, String> {
        Ok(show("DropInPaths")?
            .split_whitespace()
            .any(|entry| Path::new(entry) == path)
            && show("NeedDaemonReload")?.trim() != "yes")
    };
    let reload = changed || !loaded(&show)?;
    if reload {
        let output = run(&words(&[systemctl, "daemon-reload"]))
            .map_err(|error| format!("systemctl daemon-reload: {error}"))?;
        if !output.status.success() {
            return Err(format!("systemctl daemon-reload: {}", last_line(&output)));
        }
        if !loaded(&show)? {
            return Err(format!(
                "systemd führt {} für {unit} nach daemon-reload nicht",
                path.display()
            ));
        }
    }
    let line = text
        .lines()
        .find(|line| line.starts_with("ExecStartPre="))
        .unwrap_or_default();
    Ok(Some(format!(
        "Vorstart-Schritt für {unit} {} in {} ({}): {line}; {}",
        if changed {
            "eingerichtet"
        } else {
            "unverändert"
        },
        path.display(),
        if reload {
            "systemctl daemon-reload ausgeführt"
        } else {
            "von systemd geladen"
        },
        admit_running(&show, &exec.path)
    )))
}

/// Runs the step once now for the running instance, with its own arguments
/// and environment, so the ports open without a restart.
fn admit_running(show: &dyn Fn(&str) -> Result<String, String>, exe: &str) -> String {
    let pid = show("MainPID")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|pid| *pid > 0);
    let Some(pid) = pid else {
        return "der Dienst läuft nicht; systemd führt den Schritt bei seinem nächsten Start aus"
            .into();
    };
    let read = |name: &str| std::fs::read(format!("/proc/{pid}/{name}"));
    let (Ok(cmdline), Ok(environ)) = (read("cmdline"), read("environ")) else {
        return format!(
            "Prozess {pid} nicht lesbar; systemd führt den Schritt beim nächsten Start aus"
        );
    };
    let args = nul_fields(&cmdline)
        .into_iter()
        .skip(1)
        .filter(|arg| arg != ADMIT_FLAG);
    let environment = nul_fields(&environ).into_iter().filter_map(|pair| {
        pair.split_once('=')
            .map(|(key, value)| (key.to_string(), value.to_string()))
    });
    let status = Command::new(exe)
        .args(args)
        .arg(ADMIT_FLAG)
        .env_clear()
        .envs(environment)
        .stdin(Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => format!(
            "für den laufenden Prozess {pid} jetzt ausgeführt (Ergebnis in der firewall-Zeile davor)"
        ),
        Ok(status) => format!("Schritt für den laufenden Prozess {pid} endete mit {status}; systemd führt ihn beim nächsten Start erneut aus"),
        Err(error) => format!("{exe} nicht startbar ({error}); systemd führt den Schritt beim nächsten Start aus"),
    }
}

fn elevate(unit: &str) -> Result<(), String> {
    use std::io::IsTerminal;
    let exe = std::env::current_exe().map_err(|error| format!("Programmpfad: {error}"))?;
    let quiet = !std::io::stdin().is_terminal();
    let mut command = Command::new("sudo");
    if quiet {
        command.arg("-n");
    }
    let sudo = if quiet { "sudo -n" } else { "sudo" };
    let status = command
        .arg(&exe)
        .args([INSTALL_FLAG, "--unit", unit])
        .status()
        .map_err(|error| format!("Root-Rechte fehlen, {sudo} nicht startbar: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Root-Rechte fehlen oder die Einrichtung scheiterte ({sudo} endete mit {status})"
        ))
    }
}
