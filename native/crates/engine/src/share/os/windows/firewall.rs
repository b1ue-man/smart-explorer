//! Sets up the rules of `firewall_windows_plan` (user orders 2026-11-25:
//! „update, installation, NUTZUNG richten das ein“, „ELEVATION FEHLT?
//! ERFRAGEN!“): at the installation, after an update run by an elevated
//! helper, and at use (the Share worker's start), a missing or outdated rule
//! is created or repaired. An elevated process changes the rules itself; any
//! other asks Windows for the administrator confirmation (UAC) for exactly
//! that change. A refused confirmation is reported with the missing rules and
//! asked again at the next use; `install_gate` keeps it to one confirmation
//! at a time for this account (the desktop and the worker start together).
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

use super::firewall_windows_plan as plan;
use crate::login::os::shared::install_gate::{Attempt, Gate};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// The single-flight gate of the rule setup (`install-firewall-peer.*`).
const GATE: &str = "firewall-peer";
/// ERROR_CANCELLED: the person answered the UAC prompt with „Nein“.
const CANCELLED: i32 = 1223;
const COMMAND_LIMIT: usize = 30_000;

/// Whether a process without administrator rights asks for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consent {
    /// Ask Windows for the UAC confirmation.
    Ask,
    /// Change rules only in an elevated process, else name what is missing
    /// (the update helper: its restarted programs ask at their start).
    ElevatedOnly,
}

struct State {
    rules: Vec<plan::Observed>,
    profiles: Vec<plan::ProfileFacts>,
}

fn query() -> io::Result<State> {
    let engine = crate::net::system_powershell()?;
    let output = std::process::Command::new(engine)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &plan::query_script(),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Abfrage der Windows-Firewall endete mit {}",
            output.status
        )));
    }
    let (rules, profiles) = plan::parse_query(&String::from_utf8_lossy(&output.stdout));
    Ok(State { rules, profiles })
}

fn netsh() -> io::Result<PathBuf> {
    // <System32>\WindowsPowerShell\v1.0\powershell.exe → <System32>\netsh.exe
    crate::net::system_powershell()?
        .ancestors()
        .nth(3)
        .map(|system| system.join("netsh.exe"))
        .ok_or_else(|| io::Error::other("Systemordner unbekannt"))
}

/// The rules of the installation folder `dir` (the folder of `se.exe` and
/// `Smart Explorer.exe`); the status line when they fit.
pub fn ensure(dir: &Path, consent: Consent) -> io::Result<String> {
    let dir_text = dir
        .to_str()
        .ok_or_else(|| io::Error::other("Installationsordner ist kein Unicode-Pfad"))?;
    let expected = plan::expected_rules(dir_text, &|file| dir.join(file).is_file());
    if expected.is_empty() {
        return Err(io::Error::other(format!(
            "in {} liegt weder se.exe noch Smart Explorer.exe",
            dir.display()
        )));
    }
    let state = query()?;
    let wanted = plan::plan(&expected, &state.rules);
    if wanted.is_empty() {
        return Ok(plan::status_line(&expected, &state.profiles));
    }
    let missing = wanted.findings.join("; ");
    // A test build never changes the firewall of the machine it runs on.
    if cfg!(any(test, feature = "test-support")) {
        return Err(io::Error::other(format!(
            "Testbau ändert keine Firewall-Regeln; es fehlen: {missing}"
        )));
    }
    if crate::login::os::platform::privileged() {
        run_direct(&wanted)?;
        return verify(&expected);
    }
    if consent == Consent::ElevatedOnly {
        return Ok(format!(
            "Firewall-Regeln fehlen ({missing}); se fragt beim nächsten Start nach der Windows-Bestätigung"
        ));
    }
    let present = || {
        query()
            .map(|state| plan::plan(&expected, &state.rules).is_empty())
            .unwrap_or(false)
    };
    match Gate::named(GATE).run(&present, || elevate(&expected)) {
        Attempt::Installed => verify(&expected),
        Attempt::Declined(reason) => Err(io::Error::new(io::ErrorKind::PermissionDenied, reason)),
        Attempt::Failed(reason) => Err(io::Error::other(reason)),
    }
}

fn verify(expected: &[plan::Rule]) -> io::Result<String> {
    let state = query()?;
    let left = plan::plan(expected, &state.rules);
    if left.is_empty() {
        Ok(plan::status_line(expected, &state.profiles))
    } else {
        Err(io::Error::other(format!(
            "Firewall-Regeln nach der Einrichtung weiterhin unvollständig: {}",
            left.findings.join("; ")
        )))
    }
}

fn run_direct(wanted: &plan::Plan) -> io::Result<()> {
    let netsh = netsh()?;
    for step in &wanted.steps {
        let output = std::process::Command::new(&netsh)
            .args(step.args())
            .creation_flags(CREATE_NO_WINDOW)
            .output()?;
        // A delete may find nothing; only a failed add is an error.
        if matches!(step, plan::Step::Add(_)) && !output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            return Err(io::Error::other(format!(
                "netsh: {}",
                text.lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .unwrap_or("Regel nicht angelegt")
            )));
        }
    }
    Ok(())
}

/// The UAC step for what is still missing now (another process may have set
/// the rules meanwhile).
fn elevate(expected: &[plan::Rule]) -> Attempt {
    let current = match query() {
        Ok(state) => plan::plan(expected, &state.rules),
        Err(error) => return Attempt::Failed(format!("Abfrage der Windows-Firewall: {error}")),
    };
    if current.is_empty() {
        return Attempt::Installed;
    }
    let missing = current.findings.join("; ");
    match run_elevated(&current) {
        Ok(0) => Attempt::Installed,
        Ok(CANCELLED) => Attempt::Declined(format!(
            "Windows-Bestätigung (UAC) für die Firewall-Regeln abgelehnt; es fehlen: {missing}; se fragt beim nächsten Start erneut"
        )),
        Ok(code) => Attempt::Failed(format!(
            "Firewall-Regeln nicht eingerichtet (netsh endete mit Code {code}); es fehlen: {missing}"
        )),
        Err(error) => Attempt::Failed(format!(
            "Windows-Bestätigung konnte nicht angefragt werden: {error}; es fehlen: {missing}"
        )),
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// `Start-Process -Verb RunAs -Wait` of the system PowerShell with the
/// `netsh` steps; the exit code of the elevated step, or 1223 when the
/// prompt was refused.
fn run_elevated(wanted: &plan::Plan) -> io::Result<i32> {
    use base64::Engine;
    let netsh = netsh()?;
    let netsh = netsh
        .to_str()
        .ok_or_else(|| io::Error::other("Systempfad ist kein Unicode"))?;
    let script = plan::elevated_script(netsh, &wanted.steps);
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let engine = crate::net::system_powershell()?;
    let engine_text = engine
        .to_str()
        .ok_or_else(|| io::Error::other("Systempfad ist kein Unicode"))?;
    let launch = format!(
        "try {{ $p=Start-Process -FilePath {} -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList {}; exit $p.ExitCode }} \
         catch {{ $e=$_.Exception; while ($e) {{ if ($e.NativeErrorCode -eq {CANCELLED}) {{ exit {CANCELLED} }}; $e=$e.InnerException }}; exit 2 }}",
        quote(engine_text),
        quote(&format!(
            "-NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {encoded}"
        ))
    );
    if launch.encode_utf16().count() > COMMAND_LIMIT {
        return Err(io::Error::other(
            "Firewall-Einrichtung überschreitet die Windows-Befehlszeilengrenze",
        ));
    }
    let output = std::process::Command::new(&engine)
        .args(["-NoProfile", "-NonInteractive", "-Command", &launch])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    Ok(output.status.code().unwrap_or(2))
}
