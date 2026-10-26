//! GNOME 48+ runtime modes; never invoke `grdctl --system`, which uses pkexec.
use std::path::Path;
use crate::rdp::{core::{HostKind, HostProbe, HostService, SessionMode},
    os::shared::process::{capture, find_tool}};
use super::inventory::{self, attach_listener, certificate, service, unit};

pub(super) fn probe(report: &mut HostProbe) {
    let Some(grdctl) = find_tool(&["grdctl"]) else { return; };
    for (id, flag, mode, unit_name, user) in [
        ("gnome-console", None, SessionMode::SharedConsole, "gnome-remote-desktop.service", true),
        ("gnome-headless", Some("--headless"), SessionMode::HeadlessUserSession, "gnome-remote-desktop-headless.service", true),
        ("gnome-login", None, SessionMode::RemoteLogin, "gnome-remote-desktop.service", false),
    ] {
        let mut found = service(id, HostKind::Gnome, mode);
        found.evidence.push(format!("Installed grdctl: {}", grdctl.display()));
        match unit(unit_name, user, "gnome-remote-desktop-daemon") {
            Err(error) => found.unavailable_reason = Some(error),
            Ok(unit) => {
                if user {
                    let mut args = Vec::new();
                    if let Some(flag) = flag { args.push(flag); }
                    args.push("status");
                    match capture(&grdctl, &args) {
                        Ok(status) => apply_status(&mut found, &status, &unit),
                        Err(error) => { attach_listener(&mut found, &unit, None); found.unavailable_reason = Some(error); }
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
                        if std::fs::metadata(format!("/proc/{}", unit.pid)).ok()
                            .is_some_and(|m| m.uid() == own_uid) {
                            if let Ok(status) = capture(&grdctl, &["status"]) { apply_status(&mut found, &status, &unit); }
                        }
                    }
                }
                if unit.command.iter().any(|value| value == "--headless") && mode == SessionMode::SharedConsole {
                    found.session_mode = SessionMode::Unknown;
                    found.unavailable_reason = Some("GNOME unit command line contradicts its screen-sharing mode".into());
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
        if line == "RDP:" { rdp = true; continue; }
        if !line.starts_with(char::is_whitespace) && line.ends_with(':') { rdp = false; }
        if !rdp { continue; }
        let Some((key, value)) = line.trim().split_once(':') else { continue; };
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
            let matching: Vec<_> = ports.into_iter().filter(|p| *p >= configured && *p <= configured.saturating_add(10)).collect();
            port = if matching.len() == 1 { matching.first().copied() } else { None };
        }
    }
    attach_listener(found, unit, port);
    if port.is_none() { found.unavailable_reason = Some("GNOME effective RDP port is not proved by status and process sockets".into()); }
    if !enabled { found.unavailable_reason = Some("GNOME RDP backend is disabled or its status is unreadable".into()); }
    if let Some(cert) = cert {
        certificate(found, Path::new(&cert));
        if found.certificate_sha256.is_some() { found.tls = Some(true); }
    }
    found.evidence.push(format!("grdctl status: RDP enabled={enabled}, configured/effective port={port:?}, negotiate-port={negotiate}"));
}
