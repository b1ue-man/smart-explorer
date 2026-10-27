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
            let config = argument(&unit.command, "--config")
                .or_else(|| argument(&unit.command, "-c"))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/etc/xrdp/xrdp.ini"));
            match ini(&config) {
                Err(error) => found.unavailable_reason = Some(error),
                Ok(values) => {
                    let global = |key: &str| {
                        values
                            .get(&("Globals".into(), key.into()))
                            .map(String::as_str)
                    };
                    let configured_port = global("port").and_then(|s| s.parse::<u16>().ok());
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
