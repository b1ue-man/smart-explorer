use super::inventory::{argument, attach_listener, certificate, ini, ini_sections, service, unit};
use crate::rdp::{
    core::{HostKind, HostProbe, SessionMode},
    os::shared::process::{capture, find_tool},
};
use std::path::{Path, PathBuf};

pub(super) fn probe(report: &mut HostProbe) {
    let Some(program) = find_tool(&["xrdp"]) else {
        return;
    };
    let mut found = service("xrdp", HostKind::Xrdp, SessionMode::Unknown);
    found.version = capture(&program, &["--version"])
        .ok()
        .map(|s| s.lines().next().unwrap_or("").to_string());
    match unit("xrdp.service", false, "xrdp") {
        Err(error) => found.unavailable_reason = Some(error),
        Ok(unit) => {
            let config = config_of(&unit.command);
            match ini(&config) {
                Err(error) => found.unavailable_reason = Some(error),
                Ok(values) => {
                    let global = |key: &str| {
                        values
                            .get(&("Globals".into(), key.into()))
                            .map(String::as_str)
                    };
                    let configured_port = global("port").and_then(parse_port).map(|(port, _)| port);
                    attach_listener(&mut found, &unit, configured_port);
                    found.tls = match global("security_layer") {
                        Some("tls" | "negotiate") => Some(true),
                        Some("rdp") => Some(false),
                        _ => None,
                    };
                    let certificate_path = global("certificate")
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from)
                        .unwrap_or_else(|| {
                            config
                                .parent()
                                .unwrap_or(Path::new("/etc/xrdp"))
                                .join("cert.pem")
                        });
                    certificate(&mut found, &certificate_path);
                    // With an empty `autorun` and no login domain (se sends
                    // none) xrdp logs in with the first suitable connection
                    // section (xrdp.ini(5)); that is the one session effect.
                    let selector = global("autorun")
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .or_else(|| {
                            let first = ini_sections(&config).ok()?.into_iter().find(|section| {
                                values.contains_key(&(section.clone(), "lib".to_string()))
                            })?;
                            found.evidence.push(format!(
                                "xrdp autorun is empty and se sends no login domain: xrdp uses the first connection section {first}"
                            ));
                            Some(first)
                        });
                    let mut modes = std::collections::BTreeSet::new();
                    for ((section, key), lib) in &values {
                        if key != "lib"
                            || selector
                                .as_deref()
                                .is_some_and(|selected| selected != section)
                        {
                            continue;
                        }
                        let field = |key: &str| {
                            values
                                .get(&(section.clone(), key.into()))
                                .map(String::as_str)
                        };
                        let mode = match (lib.as_str(), field("port"), field("ip")) {
                            ("libxup.so", Some("-1"), Some("127.0.0.1")) => Some("xorg"),
                            ("libvnc.so", Some("-1"), Some("127.0.0.1")) => Some("xvnc"),
                            _ => None,
                        };
                        if let Some(mode) = mode {
                            modes.insert(mode);
                        } else {
                            modes.insert("unproved");
                        }
                        found.evidence.push(format!(
                            "xrdp login choice {section}: {}",
                            mode.unwrap_or("unproved existing/redirected backend")
                        ));
                    }
                    found.session_mode = if modes.len() == 1 && modes.contains("xorg") {
                        SessionMode::SeparateXorg
                    } else if modes.len() == 1 && modes.contains("xvnc") {
                        SessionMode::SeparateXvnc
                    } else {
                        SessionMode::Unknown
                    };
                    if found.session_mode == SessionMode::Unknown {
                        found.unavailable_reason = Some(format!(
                            "xrdp starts the session section {} which redirects to another computer; se uses only local Xorg or Xvnc sessions",
                            selector.as_deref().unwrap_or("-")
                        ));
                    }
                    found.evidence.push(format!("Read existing xrdp configuration {}; NLA and TLS settings were not changed", config.display()));
                }
            }
        }
    }
    report.services.push(found);
}

/// The `xrdp.ini` that the running unit uses (`--config`/`-c`), else the
/// packaged default.
pub(super) fn config_of(command: &[String]) -> PathBuf {
    argument(command, "--config")
        .or_else(|| argument(command, "-c"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/xrdp/xrdp.ini"))
}

/// The configuration file of the xrdp unit, also when it is not running.
pub(super) fn config_path() -> PathBuf {
    match unit("xrdp.service", false, "xrdp") {
        Ok(unit) => config_of(&unit.command),
        Err(_) => config_of(&[]),
    }
}

/// Port and loopback-only flag of one `[Globals] port=` value. Forms of
/// xrdp 0.9.24/0.10.3 (`xrdp/xrdp.ini.in`): `3389` and `tcp://:3389` listen
/// on every interface, `tcp://.:3389` means 127.0.0.1:3389. Other socket
/// kinds (vsock, unix) and lists of several listeners return `None`.
pub(crate) fn parse_port(value: &str) -> Option<(u16, bool)> {
    let value = value.trim();
    if let Ok(port) = value.parse::<u16>() {
        return Some((port, false));
    }
    let rest = value
        .strip_prefix("tcp://")
        .or_else(|| value.strip_prefix("tcp6://"))?;
    let (host, port) = rest.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    let host = host.trim_start_matches('{').trim_end_matches('}');
    let address = host.parse::<std::net::IpAddr>();
    let loopback = host == "." || address.is_ok_and(|address| address.is_loopback());
    Some((port, loopback))
}

/// Rewrites `xrdp.ini` so xrdp listens on loopback only (design 3.3): the
/// `[Globals]` port becomes `tcp://.:<p>` and `security_layer` becomes `tls`
/// unless it is `tls` or `negotiate`. A network-wide listener is changed
/// only when se installed xrdp itself; another administrator's network-wide
/// setup stays untouched and is reported. Returns the text and whether it
/// changed.
pub(crate) fn loopback_config(text: &str, se_installed: bool) -> Result<(String, bool), String> {
    const FOREIGN: &str = "xrdp_network_wide_not_se_installed";
    let mut lines: Vec<String> = Vec::new();
    let mut section = String::new();
    let mut globals_end = None;
    let (mut port_seen, mut layer_seen, mut changed) = (false, false, false);
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if section == "Globals" {
                globals_end = Some(lines.len());
            }
            section = name.to_string();
        } else if section == "Globals" && !trimmed.starts_with(['#', ';']) {
            if let Some((key, value)) = trimmed.split_once('=') {
                match key.trim() {
                    "port" => {
                        port_seen = true;
                        let (port, loopback) = parse_port(value)
                            .ok_or_else(|| format!("xrdp_port_unsupported:{}", value.trim()))?;
                        if !loopback {
                            if !se_installed {
                                return Err(FOREIGN.into());
                            }
                            lines.push(format!("port=tcp://.:{port}"));
                            changed = true;
                            continue;
                        }
                    }
                    "security_layer" => {
                        layer_seen = true;
                        if !matches!(value.trim(), "tls" | "negotiate") {
                            lines.push("security_layer=tls".into());
                            changed = true;
                            continue;
                        }
                    }
                    _ => {}
                }
            }
        }
        lines.push(line.to_string());
    }
    if section == "Globals" {
        globals_end = Some(lines.len());
    }
    let end = globals_end.ok_or_else(|| "xrdp.ini has no [Globals] section".to_string())?;
    let mut missing = Vec::new();
    if !port_seen {
        // Without `port` xrdp listens on 3389 on every interface.
        if !se_installed {
            return Err(FOREIGN.into());
        }
        missing.push("port=tcp://.:3389".to_string());
    }
    if !layer_seen {
        missing.push("security_layer=tls".to_string());
    }
    changed |= !missing.is_empty();
    lines.splice(end..end, missing);
    Ok((lines.join("\n") + "\n", changed))
}
