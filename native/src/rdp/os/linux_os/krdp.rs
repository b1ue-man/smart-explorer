use super::inventory::{argument, attach_listener, certificate, service, unit};
use crate::rdp::{
    core::{HostKind, HostProbe, SessionMode},
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
