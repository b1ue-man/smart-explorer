//! Elevated helper for Internet Connection Sharing on Windows.
//!
//! One-time setup (one UAC prompt) registers a Scheduled Task that runs
//! `se.exe --lan-uplink-helper` with highest privileges on demand and makes
//! the `SharedAccess` service startable. Afterwards the daemon writes a
//! request file, starts the task unelevated, and waits for the response file.
//! The helper re-validates every request against its own view of the
//! network, so a stray request can never enable sharing on a routed link.
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::net::{classify_links, LinkClass, UplinkTarget};

pub(crate) const HELPER_MODE: &str = "--lan-uplink-helper";
const TASK_NAME: &str = "Smart Explorer LAN-Uplink";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const REQUEST_MAX_AGE_SECS: i64 = 60;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HelperOp {
    Enable,
    Disable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HelperRequest {
    pub op: HelperOp,
    pub public_guid: String,
    pub private_guid: String,
    pub issued_at: i64,
    pub nonce: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HelperResponse {
    pub nonce: String,
    pub ok: bool,
    pub message: String,
}

fn directory() -> PathBuf {
    crate::support_dirs::app_data_dir().join("lan_uplink")
}

fn request_path() -> PathBuf {
    directory().join("request.json")
}

fn response_path() -> PathBuf {
    directory().join("response.json")
}

fn powershell(args: &[&str]) -> io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"])
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
}

fn escape_single(value: &str) -> String {
    value.replace('\'', "''")
}

/// Whether the scheduled task exists for the current user.
pub(crate) fn task_registered() -> io::Result<bool> {
    let output = powershell(&[&format!(
        "if (Get-ScheduledTask -TaskName '{}' -ErrorAction SilentlyContinue) {{ 'yes' }} else {{ 'no' }}",
        escape_single(TASK_NAME)
    )])?;
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "yes")
}

/// One UAC prompt: register the on-demand task bound to this executable and
/// make the ICS service startable. The script is staged as a file so no
/// quoting round-trips through `Start-Process`. Blocks until the elevated
/// script exits.
pub(crate) fn setup_once() -> io::Result<String> {
    use std::os::windows::process::CommandExt;
    let exe = std::env::current_exe()?;
    let exe = exe.to_string_lossy().to_string();
    std::fs::create_dir_all(directory())?;
    let script_path = directory().join("setup.ps1");
    let script = format!(
        "$ErrorActionPreference = 'Stop'\r\n\
         $action = New-ScheduledTaskAction -Execute '{exe}' -Argument '{HELPER_MODE}'\r\n\
         $principal = New-ScheduledTaskPrincipal -UserId \"$env:USERDOMAIN\\$env:USERNAME\" -LogonType Interactive -RunLevel Highest\r\n\
         $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Minutes 2)\r\n\
         Register-ScheduledTask -TaskName '{task}' -Action $action -Principal $principal -Settings $settings -Force | Out-Null\r\n\
         $svc = Get-Service -Name SharedAccess -ErrorAction Stop\r\n\
         if ($svc.StartType -eq 'Disabled') {{ Set-Service -Name SharedAccess -StartupType Manual }}\r\n",
        exe = escape_single(&exe),
        task = escape_single(TASK_NAME),
    );
    std::fs::write(&script_path, script)?;
    let argument_list = format!(
        "-NoProfile -ExecutionPolicy Bypass -File \"{}\"",
        script_path.to_string_lossy()
    );
    let status = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &format!(
                "$p = Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -PassThru -ArgumentList '{}'; exit $p.ExitCode",
                escape_single(&argument_list)
            ),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    let _ = std::fs::remove_file(&script_path);
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Windows-UAC-Freigabe wurde abgelehnt oder die Einrichtung schlug fehl",
        ));
    }
    if !task_registered()? {
        return Err(io::Error::other(
            "Aufgabe 'Smart Explorer LAN-Uplink' wurde nicht angelegt",
        ));
    }
    Ok("Einmalige Einrichtung abgeschlossen: Aufgabenplanung und ICS-Dienst bereit".into())
}

/// Hand one operation to the elevated task and wait for its answer.
pub(crate) fn run_via_task(op: HelperOp, public: &UplinkTarget, private: &UplinkTarget) -> io::Result<()> {
    if !crate::net::valid_adapter_id(&public.adapter_id)
        || !crate::net::valid_adapter_id(&private.adapter_id)
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Adapter-GUID ist ungueltig"));
    }
    std::fs::create_dir_all(directory())?;
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let request = HelperRequest {
        op,
        public_guid: public.adapter_id.clone(),
        private_guid: private.adapter_id.clone(),
        issued_at: crate::share::core_now_secs(),
        nonce: nonce.clone(),
    };
    let _ = std::fs::remove_file(response_path());
    std::fs::write(
        request_path(),
        serde_json::to_string(&request).map_err(io::Error::other)?,
    )?;
    let output = powershell(&[&format!(
        "Start-ScheduledTask -TaskName '{}'",
        escape_single(TASK_NAME)
    )])?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Aufgabe konnte nicht gestartet werden: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let deadline = Instant::now() + RESPONSE_TIMEOUT;
    loop {
        if let Ok(text) = std::fs::read_to_string(response_path()) {
            if let Ok(response) = serde_json::from_str::<HelperResponse>(&text) {
                if response.nonce == nonce {
                    let _ = std::fs::remove_file(response_path());
                    return if response.ok {
                        Ok(())
                    } else {
                        Err(io::Error::other(response.message))
                    };
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Der elevierte Helfer hat nicht geantwortet (Aufgabe nicht angelegt oder blockiert)",
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Entry point of `se.exe --lan-uplink-helper` (runs elevated).
pub(crate) fn run_helper() -> io::Result<()> {
    let text = std::fs::read_to_string(request_path())?;
    let request: HelperRequest = serde_json::from_str(&text).map_err(io::Error::other)?;
    let result = validate_and_apply(&request);
    let response = HelperResponse {
        nonce: request.nonce.clone(),
        ok: result.is_ok(),
        message: match &result {
            Ok(message) => message.clone(),
            Err(error) => error.to_string(),
        },
    };
    let _ = std::fs::remove_file(request_path());
    std::fs::write(
        response_path(),
        serde_json::to_string(&response).map_err(io::Error::other)?,
    )?;
    result.map(|_| ())
}

fn validate_and_apply(request: &HelperRequest) -> io::Result<String> {
    let now = crate::share::core_now_secs();
    if now.saturating_sub(request.issued_at).abs() > REQUEST_MAX_AGE_SECS {
        return Err(io::Error::other("Anforderung ist veraltet"));
    }
    if !crate::net::valid_adapter_id(&request.public_guid)
        || !crate::net::valid_adapter_id(&request.private_guid)
        || request.public_guid.eq_ignore_ascii_case(&request.private_guid)
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Adapter-GUIDs sind ungueltig"));
    }
    let settings = crate::share::LanSettings::load().map_err(io::Error::other)?;
    match request.op {
        HelperOp::Enable => {
            if !settings.uplink_sharing_enabled {
                return Err(io::Error::other(
                    "Internet-Teilen ist in den Einstellungen ausgeschaltet",
                ));
            }
            let facts = crate::net::gather_interface_facts().map_err(io::Error::other)?;
            let links = classify_links(&facts, &[], None, &[]);
            let private_ok = links.iter().any(|(iface, class)| {
                iface.adapter_id.eq_ignore_ascii_case(&request.private_guid)
                    && *class == LinkClass::RouterLess
            });
            if !private_ok {
                return Err(io::Error::other(
                    "Der private Adapter ist kein Link ohne Router; Freigabe verweigert",
                ));
            }
            let public_ok = links.iter().any(|(iface, class)| {
                iface.adapter_id.eq_ignore_ascii_case(&request.public_guid)
                    && matches!(class, LinkClass::Uplink | LinkClass::Routed)
            });
            if !public_ok {
                return Err(io::Error::other(
                    "Der oeffentliche Adapter hat keinen Gateway; Freigabe verweigert",
                ));
            }
            super::ics::enable_sharing(&request.public_guid, &request.private_guid)?;
            Ok("Internetverbindungsfreigabe aktiviert".into())
        }
        HelperOp::Disable => {
            super::ics::disable_sharing(&request.public_guid, &request.private_guid)?;
            Ok("Internetverbindungsfreigabe deaktiviert".into())
        }
    }
}
