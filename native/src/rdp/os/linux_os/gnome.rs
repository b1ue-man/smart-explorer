//! GNOME 48+ runtime modes; never invoke `grdctl --system`, which uses pkexec.
use super::inventory::{self, attach_listener, certificate, service, unit};
use crate::rdp::{
    core::{HostKind, HostProbe, HostService, SessionMode, SetupState},
    os::shared::process::{capture, find_tool},
};
use std::path::Path;

pub(super) fn probe(report: &mut HostProbe) {
    let Some(grdctl) = find_tool(&["grdctl"]) else {
        return;
    };
    for (id, flag, mode, unit_name, user) in [
        (
            "gnome-console",
            None,
            SessionMode::SharedConsole,
            "gnome-remote-desktop.service",
            true,
        ),
        (
            "gnome-headless",
            Some("--headless"),
            SessionMode::HeadlessUserSession,
            "gnome-remote-desktop-headless.service",
            true,
        ),
        (
            "gnome-login",
            None,
            SessionMode::RemoteLogin,
            "gnome-remote-desktop.service",
            false,
        ),
    ] {
        let mut found = service(id, HostKind::Gnome, mode);
        found
            .evidence
            .push(format!("Installed grdctl: {}", grdctl.display()));
        // se hands out the credentials of the screen-sharing backend it set up.
        found.managed_credentials = id == "gnome-console" && credentials_managed();
        match unit(unit_name, user, "gnome-remote-desktop-daemon") {
            Err(error) => found.unavailable_reason = Some(error),
            Ok(unit) => {
                if user {
                    let mut args = Vec::new();
                    if let Some(flag) = flag {
                        args.push(flag);
                    }
                    args.push("status");
                    match capture(&grdctl, &args) {
                        Ok(status) => apply_status(&mut found, &status, &unit),
                        Err(error) => {
                            attach_listener(&mut found, &unit, None);
                            found.unavailable_reason = Some(error);
                        }
                    }
                } else {
                    // The system unit proves Remote Login, independently from
                    // user screen sharing. No escalation or daemon mutation.
                    attach_listener(&mut found, &unit, None);
                    if unit.active {
                        found.evidence.push("GNOME system remote-login unit; grdctl --system was not invoked because it launches pkexec".into());
                        found.tls = Some(true);
                        found.evidence.push("The system daemon certificate is not accessible through an unprivileged grdctl status; a privileged host inventory must supply its certificate fingerprint before start".into());
                        // A service-user daemon owner can obtain system status
                        // directly: grdctl detects that UID without --system.
                        use std::os::unix::fs::MetadataExt;
                        // SAFETY: geteuid takes no pointer and has no side effects.
                        let own_uid = unsafe { libc::geteuid() };
                        if std::fs::metadata(format!("/proc/{}", unit.pid))
                            .ok()
                            .is_some_and(|m| m.uid() == own_uid)
                        {
                            if let Ok(status) = capture(&grdctl, &["status"]) {
                                apply_status(&mut found, &status, &unit);
                            }
                        }
                    }
                }
                if unit.command.iter().any(|value| value == "--headless")
                    && mode == SessionMode::SharedConsole
                {
                    found.session_mode = SessionMode::Unknown;
                    found.unavailable_reason =
                        Some("GNOME unit command line contradicts its screen-sharing mode".into());
                }
            }
        }
        report.services.push(found);
    }
}
fn apply_status(found: &mut HostService, status: &str, unit: &inventory::Unit) {
    let mut rdp = false;
    let mut port = None;
    let mut enabled = false;
    let mut cert = None;
    let mut negotiate = false;
    for line in status.lines() {
        if line == "RDP:" {
            rdp = true;
            continue;
        }
        if !line.starts_with(char::is_whitespace) && line.ends_with(':') {
            rdp = false;
        }
        if !rdp {
            continue;
        }
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "Status" => enabled = value == "enabled",
            "Port" => port = value.parse::<u16>().ok().filter(|p| *p != 0),
            "TLS certificate" if !value.is_empty() => cert = Some(value.to_string()),
            "Negotiate port" => negotiate = value == "yes",
            _ => {}
        }
    }
    // Port negotiation can select the next ten ports. Require a single
    // process-owned candidate from that actual range; no guessed port 3389.
    if negotiate {
        if let (Some(configured), Ok(ports)) = (port, inventory::listeners(unit.pid)) {
            let matching: Vec<_> = ports
                .into_iter()
                .filter(|p| *p >= configured && *p <= configured.saturating_add(10))
                .collect();
            port = if matching.len() == 1 {
                matching.first().copied()
            } else {
                None
            };
        }
    }
    attach_listener(found, unit, port);
    if port.is_none() {
        found.unavailable_reason =
            Some("GNOME effective RDP port is not proved by status and process sockets".into());
    }
    if !enabled {
        found.unavailable_reason =
            Some("GNOME RDP backend is disabled or its status is unreadable".into());
    }
    if let Some(cert) = cert {
        certificate(found, Path::new(&cert));
        if found.certificate_sha256.is_some() {
            found.tls = Some(true);
        }
    }
    found.evidence.push(format!("grdctl status: RDP enabled={enabled}, configured/effective port={port:?}, negotiate-port={negotiate}"));
}

/// Marker of a GNOME desktop sharing whose RDP credentials se manages.
fn managed_marker() -> std::path::PathBuf {
    crate::support_dirs::app_data_dir()
        .join("rdp")
        .join("gnome-managed")
}

/// Whether se set GNOME desktop sharing up and hands out its credentials.
pub(super) fn credentials_managed() -> bool {
    managed_marker().is_file()
}

/// The fixed RDP user name of this host: `se-` and 8 hex digits of the
/// SHA-256 of its device ID (design 3.3 step 4).
pub(crate) fn managed_username(device_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(device_id.as_bytes());
    let prefix = [digest[0], digest[1], digest[2], digest[3]];
    format!("se-{:08x}", u32::from_be_bytes(prefix))
}

/// 32 random bytes, base64url without padding (256 bits, threat T-R2).
fn random_password() -> Result<String, String> {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| format!("random: {error}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// The `stdin` lines `grdctl rdp set-credentials` reads when user name and
/// password are not arguments (GNOME 48.2 `src/grd-ctl.c`); nothing reaches
/// argv, files or logs.
pub(crate) fn credential_input(username: &str, password: &str) -> String {
    format!("{username}\n{password}\n")
}

fn grdctl() -> Result<std::path::PathBuf, SetupState> {
    find_tool(&["grdctl"]).ok_or_else(|| SetupState::Failed("grdctl_missing".into()))
}

/// A failed `grdctl` step as setup state; a locked or unreachable secret
/// store is `keyring_locked` (design 3.4, retried at T3/T4).
fn grdctl_failure(step: &str, error: String) -> SetupState {
    let lower = error.to_ascii_lowercase();
    let keyring = ["keyring", "secret", "locked", "exceeded"];
    if keyring.iter().any(|word| lower.contains(word)) {
        SetupState::Failed("keyring_locked".into())
    } else {
        SetupState::Failed(format!("grdctl_{step}: {error}"))
    }
}

fn grdctl_run(grdctl: &Path, args: &[&str], input: Option<&[u8]>) -> Result<String, SetupState> {
    let step = args.last().copied().unwrap_or("status");
    super::host_setup::run_tool(grdctl, args, input).map_err(|error| grdctl_failure(step, error))
}

fn set_credentials(username: &str, password: &str) -> Result<(), SetupState> {
    let input = credential_input(username, password);
    let args = ["rdp", "set-credentials"];
    grdctl_run(&grdctl()?, &args, Some(input.as_bytes())).map(|_| ())
}

/// The configured TLS certificate of the user RDP backend, if any.
fn configured_certificate(status: &str) -> Option<String> {
    let mut rdp = false;
    for line in status.lines() {
        if line == "RDP:" {
            rdp = true;
            continue;
        }
        if !line.starts_with(char::is_whitespace) && line.ends_with(':') {
            rdp = false;
        }
        if let Some(value) = line.trim().strip_prefix("TLS certificate:") {
            let value = value.trim();
            if rdp && !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn tls_failure(step: &str, error: impl std::fmt::Display) -> SetupState {
    SetupState::Failed(format!("gnome_tls_{step}: {error}"))
}

/// A self-signed certificate with rcgen's own validity defaults (no se
/// lifetime), key file 0600 (design 3.3 step 1).
fn ensure_tls(grdctl: &Path) -> Result<(), SetupState> {
    let status = grdctl_run(grdctl, &["status"], None)?;
    let configured = configured_certificate(&status);
    if configured.is_some_and(|path| Path::new(&path).is_file()) {
        return Ok(());
    }
    let dir = crate::support_dirs::app_data_dir().join("rdp");
    crate::support_dirs::ensure_private_dir(&dir).map_err(|e| tls_failure("dir", e))?;
    let crt = dir.join("gnome-tls.crt");
    let key = dir.join("gnome-tls.key");
    let pair = rcgen::KeyPair::generate().map_err(|e| tls_failure("key", e))?;
    let names = vec!["localhost".to_string()];
    let params = rcgen::CertificateParams::new(names).map_err(|e| tls_failure("params", e))?;
    let certificate = params
        .self_signed(&pair)
        .map_err(|e| tls_failure("sign", e))?;
    let key_pem = pair.serialize_pem();
    crate::support_dirs::write_private_atomic(&key, key_pem.as_bytes())
        .map_err(|e| tls_failure("key_file", e))?;
    crate::support_dirs::write_private_atomic(&crt, certificate.pem().as_bytes())
        .map_err(|e| tls_failure("cert_file", e))?;
    for (command, path) in [("set-tls-cert", &crt), ("set-tls-key", &key)] {
        let path = path
            .to_str()
            .ok_or_else(|| tls_failure("path", "not UTF-8"))?;
        grdctl_run(grdctl, &["rdp", command, path], None)?;
    }
    Ok(())
}

/// GNOME desktop sharing set up by the session's own service (design 3.3):
/// TLS, RDP enabled with control, the user unit running, fresh random
/// credentials that nobody holds yet.
pub(super) fn configure(device_id: &str) -> SetupState {
    match configure_steps(device_id) {
        Ok(()) => SetupState::Done,
        Err(state) => state,
    }
}

fn configure_steps(device_id: &str) -> Result<(), SetupState> {
    let grdctl = grdctl()?;
    ensure_tls(&grdctl)?;
    grdctl_run(&grdctl, &["rdp", "enable"], None)?;
    grdctl_run(&grdctl, &["rdp", "disable-view-only"], None)?;
    let systemctl =
        find_tool(&["systemctl"]).ok_or_else(|| SetupState::Failed("systemctl_missing".into()))?;
    let unit = ["--user", "enable", "--now", "gnome-remote-desktop.service"];
    super::host_setup::run_tool(&systemctl, &unit, None)
        .map_err(|e| SetupState::Failed(format!("gnome_unit: {e}")))?;
    let password = random_password().map_err(SetupState::Failed)?;
    set_credentials(&managed_username(device_id), &password)?;
    crate::support_dirs::write_private_atomic(&managed_marker(), b"")
        .map_err(|e| SetupState::Failed(format!("gnome_marker: {e}")))
}

/// New credentials for one RDP start (`RdpCredential`); se keeps no copy.
pub(super) fn issue_credential(device_id: &str) -> Result<(String, String), String> {
    let username = managed_username(device_id);
    let password = random_password()?;
    set_credentials(&username, &password).map_err(|state| state.text())?;
    Ok((username, password))
}

/// New random credentials that nobody receives (after a revocation).
pub(super) fn rotate_credential(device_id: &str) -> Result<(), String> {
    if !credentials_managed() {
        return Ok(());
    }
    let password = random_password()?;
    let username = managed_username(device_id);
    set_credentials(&username, &password).map_err(|state| state.text())
}
