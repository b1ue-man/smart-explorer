use crate::rdp::{
    core::{CertificateFingerprint, CertificateSha1, HostKind, HostService, SessionMode},
    os::shared::process::{capture, find_tool, read_text},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub(super) struct Unit {
    pub(super) active: bool,
    pub(super) pid: u32,
    pub(super) command: Vec<String>,
}
pub(super) fn unit(name: &str, user: bool, executable: &str) -> Result<Unit, String> {
    let systemctl =
        find_tool(&["systemctl"]).ok_or_else(|| "systemctl is unavailable".to_string())?;
    let mut args = Vec::new();
    if user {
        args.push("--user");
    }
    args.extend(["show", "--no-pager", "--property=ActiveState,MainPID", name]);
    let text = capture(&systemctl, &args)?;
    let active = text.lines().any(|line| line == "ActiveState=active");
    let pid = text
        .lines()
        .find_map(|line| line.strip_prefix("MainPID=")?.parse::<u32>().ok())
        .ok_or_else(|| format!("{name} has no process inventory"))?;
    let command = if active && pid != 0 {
        let image = std::fs::read_link(format!("/proc/{pid}/exe"))
            .map_err(|e| format!("Cannot identify {name} process: {e}"))?;
        if image.file_name().and_then(|s| s.to_str()) != Some(executable) {
            return Err(format!("{name} process image does not match {executable}"));
        }
        let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).map_err(|e| e.to_string())?;
        if bytes.len() > 128 * 1024 {
            return Err("Service command line exceeds inventory bound".into());
        }
        bytes
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| {
                String::from_utf8(s.to_vec()).map_err(|_| "Service argv is not UTF-8".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    Ok(Unit {
        active,
        pid,
        command,
    })
}

/// Match listening socket inodes to this process's own descriptors. A port
/// merely found somewhere on the machine is never evidence of this service.
/// Without access to another account's descriptors (a user service probing
/// root's xrdp, design 3.3) a LISTEN socket of the service's own UID on a
/// loopback or wildcard address is the proof.
pub(super) fn listeners(pid: u32) -> Result<Vec<u16>, String> {
    if pid == 0 {
        return Ok(Vec::new());
    }
    let fds = match std::fs::read_dir(format!("/proc/{pid}/fd")) {
        Ok(fds) => fds,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            use std::os::unix::fs::MetadataExt;
            let uid = std::fs::metadata(format!("/proc/{pid}"))
                .map_err(|e| format!("Cannot identify the RDP service account: {e}"))?
                .uid();
            return crate::share::os_sockets::loopback_listen_ports_of_uid(uid)
                .map_err(|e| format!("Cannot read the TCP listener table: {e}"));
        }
        Err(error) => return Err(format!("Cannot inspect RDP service sockets: {error}")),
    };
    let mut inodes = BTreeSet::new();
    for entry in fds.take(65536) {
        if let Ok(target) = entry
            .map_err(|e| e.to_string())
            .and_then(|e| std::fs::read_link(e.path()).map_err(|e| e.to_string()))
        {
            let target = target.to_string_lossy();
            if let Some(inode) = target
                .strip_prefix("socket:[")
                .and_then(|s| s.strip_suffix(']'))
            {
                inodes.insert(inode.to_string());
            }
        }
    }
    let mut ports = BTreeSet::new();
    for family in ["tcp", "tcp6"] {
        let text = read_text(Path::new(&format!("/proc/{pid}/net/{family}")))?;
        for line in text.lines().skip(1) {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 10 || fields[3] != "0A" || !inodes.contains(fields[9]) {
                continue;
            }
            let Some((address, port)) = fields[1].rsplit_once(':') else {
                continue;
            };
            let loopback_or_any = match family {
                "tcp" => address == "00000000" || address == "0100007F",
                _ => {
                    address == "00000000000000000000000000000000"
                        || address == "00000000000000000000000001000000"
                }
            };
            if loopback_or_any {
                if let Ok(port) = u16::from_str_radix(port, 16) {
                    if port != 0 {
                        ports.insert(port);
                    }
                }
            }
        }
    }
    Ok(ports.into_iter().collect())
}
pub(super) fn service(id: &str, kind: HostKind, mode: SessionMode) -> HostService {
    HostService {
        id: id.into(),
        kind,
        version: None,
        session_mode: mode,
        port: None,
        running: false,
        tls: None,
        nla: None,
        certificate_sha256: None,
        certificate_source: None,
        remote_apps: Vec::new(),
        evidence: Vec::new(),
        unavailable_reason: None,
        certificate_sha1: None,
        managed_credentials: false,
        listener_pid: None,
    }
}
pub(super) fn attach_listener(service: &mut HostService, unit: &Unit, expected: Option<u16>) {
    service.running = unit.active;
    service.evidence.push(format!(
        "systemd MainPID={} active={}",
        unit.pid, unit.active
    ));
    if !unit.active {
        service.unavailable_reason = Some("The installed RDP service is not active".into());
        return;
    }
    match listeners(unit.pid) {
        Ok(ports) => {
            service.port = match expected {
                Some(expected) => ports.iter().find(|p| **p == expected).copied(),
                None if ports.len() == 1 => ports.first().copied(),
                None => None,
            };
            service.evidence.push(format!(
                "Process-owned loopback-reachable TCP listeners: {ports:?}"
            ));
            if service.port.is_none() {
                service.unavailable_reason = Some("The configured RDP port cannot be bound to one unambiguous process-owned listener".into());
            } else {
                service.listener_pid = Some(unit.pid);
            }
        }
        Err(error) => service.unavailable_reason = Some(error),
    }
}
/// The digest of one `openssl x509 -noout -<digest> -fingerprint` output
/// line (`SHA256 Fingerprint=AB:CD:…`, `SHA1 Fingerprint=…`).
pub(super) fn fingerprint_digest(text: &str) -> Result<&str, String> {
    text.trim()
        .split_once('=')
        .map(|(_, digest)| digest)
        .ok_or_else(|| "X.509 certificate fingerprint output is invalid".to_string())
}
/// The SHA-256 pin (FreeRDP) and the SHA-1 thumbprint (mstsc `CertHash`,
/// design 2.7.2) of the one configured certificate, both read by `openssl`
/// from the same file. A missing SHA-1 alone keeps the SHA-256 pin; the
/// reason lands in the evidence.
pub(super) fn certificate(service: &mut HostService, path: &Path) {
    let fingerprint = |digest: &str| -> Result<String, String> {
        if !path.is_absolute() {
            return Err("RDP certificate inventory path is not absolute".into());
        }
        let openssl = find_tool(&["openssl"]).ok_or_else(|| {
            "openssl is unavailable for reading the configured X.509 certificate".to_string()
        })?;
        let path_text = path
            .to_str()
            .ok_or_else(|| "RDP certificate path is not UTF-8".to_string())?;
        let args = ["x509", "-noout", digest, "-fingerprint", "-in", path_text];
        let text = capture(&openssl, &args)?;
        fingerprint_digest(&text).map(str::to_string)
    };
    match fingerprint("-sha256").and_then(|digest| CertificateFingerprint::parse(&digest)) {
        Ok(pin) => {
            service.certificate_sha256 = Some(pin);
            service.certificate_source = Some(format!(
                "Configured X.509 certificate {} (SHA-256 and SHA-1 of DER; no private key read)",
                path.display()
            ));
        }
        Err(error) => {
            service.evidence.push(error);
            return;
        }
    }
    match fingerprint("-sha1").and_then(|digest| CertificateSha1::parse(&digest)) {
        Ok(thumbprint) => service.certificate_sha1 = Some(thumbprint),
        Err(error) => service.evidence.push(format!("SHA-1 thumbprint: {error}")),
    }
}
pub(super) fn ini(path: &Path) -> Result<BTreeMap<(String, String), String>, String> {
    let text = read_text(path)?;
    let mut section = String::new();
    let mut values = BTreeMap::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.to_string();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            values.insert((section.clone(), key.trim().into()), value.trim().into());
        }
    }
    Ok(values)
}
/// The section names of an ini file in file order.
pub(super) fn ini_sections(path: &Path) -> Result<Vec<String>, String> {
    let text = read_text(path)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix('[').and_then(|s| s.strip_suffix(']')))
        .map(str::to_string)
        .collect())
}
pub(super) fn argument<'a>(command: &'a [String], name: &str) -> Option<&'a str> {
    command.iter().enumerate().find_map(|(index, value)| {
        if value == name {
            command.get(index + 1).map(String::as_str)
        } else {
            value.strip_prefix(name).and_then(|s| s.strip_prefix('='))
        }
    })
}

#[cfg(test)]
#[path = "inventory_cert_tests.rs"]
mod einrichtung_review_cert_tests;
