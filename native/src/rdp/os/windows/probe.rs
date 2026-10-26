use crate::rdp::{
    core::{
        CertificateFingerprint, HostKind, HostProbe, HostService, PublishedApp, SessionMode,
        RDP_PROBE_VERSION,
    },
    os::shared::process::capture,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Inventory {
    supported: bool,
    home: bool,
    server: bool,
    edition: String,
    sku: u32,
    version: String,
    enabled: bool,
    running: bool,
    port: Option<u16>,
    tls: Option<bool>,
    nla: Option<bool>,
    certificate_sha256: Option<CertificateFingerprint>,
    certificate_source: Option<String>,
    remote_apps: Vec<PublishedApp>,
    notes: Vec<String>,
}
pub(crate) fn probe() -> HostProbe {
    let result = (|| {
        let system = super::clients::system_windows_directory()
            .ok_or_else(|| "Windows system directory is unavailable".to_string())?;
        let powershell = system.join("WindowsPowerShell/v1.0/powershell.exe");
        let output = capture(
            &powershell,
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                include_str!("probe.ps1"),
            ],
        )?;
        serde_json::from_str::<Inventory>(output.trim_start_matches('\u{feff}'))
            .map_err(|error| format!("Windows RDP inventory is invalid: {error}"))
    })();
    let inventory = match result {
        Ok(inventory) => inventory,
        Err(error) => return HostProbe::unavailable("windows", &error),
    };
    if !inventory.supported {
        let reason = if inventory.home {
            "Windows Home/Core cannot host Remote Desktop".into()
        } else {
            format!(
                "Windows edition {} (SKU {}) has no reviewed RDP host capability",
                inventory.edition, inventory.sku
            )
        };
        return HostProbe::unavailable("windows", &reason);
    }
    let unavailable_reason = if !inventory.enabled {
        Some("Windows Remote Desktop is disabled; it is not enabled automatically".into())
    } else if !inventory.running || inventory.port.is_none() {
        Some("No TermService-owned TCP listener was proved; required inventory permissions may be missing".into())
    } else {
        None
    };
    let service = HostService { id: "windows-rdp".into(),
        kind: if inventory.server { HostKind::WindowsServer } else { HostKind::WindowsWorkstation },
        version: Some(inventory.version), session_mode: if inventory.server { SessionMode::WindowsServerSession }
            else { SessionMode::WindowsConsoleLocked },
        port: inventory.port, running: inventory.running, tls: inventory.tls, nla: inventory.nla,
        certificate_sha256: inventory.certificate_sha256, certificate_source: inventory.certificate_source,
        remote_apps: inventory.remote_apps, unavailable_reason,
        evidence: vec![format!("Win32_OperatingSystem: {} SKU={} server={}", inventory.edition, inventory.sku, inventory.server),
            "Read-only Terminal Server settings, TermService socket ownership and Win32_TSGeneralSetting; TLS/NLA/firewall unchanged".into()] };
    let mut report = HostProbe {
        version: RDP_PROBE_VERSION,
        platform: "windows".into(),
        services: vec![service],
        unavailable_reason: None,
        diagnostics: inventory.notes,
    };
    // A duplicate alias across collections is ambiguous without broker routing.
    let mut aliases = std::collections::BTreeMap::new();
    for app in &report.services[0].remote_apps {
        *aliases.entry(app.alias.clone()).or_insert(0usize) += 1;
    }
    report.services[0]
        .remote_apps
        .retain(|app| aliases.get(&app.alias) == Some(&1));
    if aliases.values().any(|count| *count > 1) {
        report.diagnostics.push("Ambiguous aliases published by multiple RDS collections were omitted; no collection is guessed".into());
    }
    report
}
