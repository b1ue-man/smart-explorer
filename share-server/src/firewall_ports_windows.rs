//! The Windows part of `firewall_ports`: one program-bound inbound allow rule
//! of this program per protocol it listens on, in every network profile. An
//! administrator or the service account sets them directly; a console
//! without administrator rights asks through the UAC prompt.
use super::{fits, last_line, netsh_steps, plain, Listener, Proto, Refusal};
use std::io::IsTerminal;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// ERROR_CANCELLED: the UAC prompt was answered with „Nein“.
const CANCELLED: i32 = 1223;

const QUERY: &str = "[Console]::OutputEncoding=[Text.Encoding]::UTF8; $t=[char]9; \
    Get-NetFirewallRule -DisplayName 'se-share-server *' -ErrorAction SilentlyContinue | ForEach-Object { \
    $a=$_ | Get-NetFirewallApplicationFilter; $p=$_ | Get-NetFirewallPortFilter; \
    (@($_.DisplayName,$_.Enabled,$_.Direction,$_.Action,$_.Profile,$p.Protocol,$a.Program) -join $t) }";

fn system32() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    PathBuf::from(root).join("System32")
}

fn powershell(script: &str) -> std::io::Result<std::process::Output> {
    Command::new(system32().join(r"WindowsPowerShell\v1.0\powershell.exe"))
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
}

fn current_fits(program: &str, protos: &[Proto]) -> bool {
    powershell(QUERY).is_ok_and(|out| fits(&String::from_utf8_lossy(&out.stdout), program, protos))
}

pub(super) fn admit(ports: &[Listener]) -> Result<String, Refusal> {
    let exe = std::env::current_exe().map_err(|error| format!("Programmpfad: {error}"))?;
    let program = plain(&exe.to_string_lossy());
    let mut protos: Vec<Proto> = Vec::new();
    for port in ports {
        if !protos.contains(&port.proto) {
            protos.push(port.proto);
        }
    }
    let done = || {
        format!(
            "Windows-Firewall: Regeln aktiv für {program} ({})",
            super::labels(ports)
        )
    };
    if current_fits(&program, &protos) {
        return Ok(done());
    }
    let steps = netsh_steps(&program, &protos);
    let netsh = system32().join("netsh.exe");
    let mut refused = String::new();
    for step in &steps {
        let output = Command::new(&netsh)
            .args(step)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|error| format!("netsh: {error}"))?;
        if step[2] == "add" && !output.status.success() {
            refused = last_line(&output);
            break;
        }
    }
    if refused.is_empty() && current_fits(&program, &protos) {
        return Ok(done());
    }
    if !std::io::stdin().is_terminal() {
        return Err(Refusal::rights(format!(
            "Administratorrechte fehlen (netsh: {refused})"
        )));
    }
    let line: Vec<String> = steps
        .iter()
        .map(|step| {
            let words: Vec<String> = step
                .iter()
                .map(|word| match word.split_once('=') {
                    Some((key, value)) if value.contains(' ') => format!("{key}=\"{value}\""),
                    _ => word.clone(),
                })
                .collect();
            format!("netsh {}", words.join(" "))
        })
        .collect();
    let quote = |value: &str| format!("'{}'", value.replace('\'', "''"));
    let script = format!(
        "try {{ $p=Start-Process -FilePath {} -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList {}; exit $p.ExitCode }} \
         catch {{ $e=$_.Exception; while ($e) {{ if ($e.NativeErrorCode -eq {CANCELLED}) {{ exit {CANCELLED} }}; $e=$e.InnerException }}; exit 2 }}",
        quote(&system32().join("cmd.exe").to_string_lossy()),
        quote(&format!("/c {}", line.join(" & ")))
    );
    let code = powershell(&script)
        .map_err(|error| format!("UAC-Abfrage: {error}"))?
        .status
        .code();
    if current_fits(&program, &protos) {
        Ok(done())
    } else if code == Some(CANCELLED) {
        Err(Refusal::rights(
            "Windows-Bestätigung (UAC) abgelehnt; se-share-server fragt beim nächsten Start erneut"
                .into(),
        ))
    } else {
        Err(format!("Firewall-Regeln nicht gesetzt (netsh: {refused})").into())
    }
}
