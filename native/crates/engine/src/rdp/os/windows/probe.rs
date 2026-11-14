use crate::rdp::{
    core::{
        CertificateFingerprint, CertificateSha1, HostKind, HostProbe, HostService, PublishedApp,
        SessionMode, RDP_PROBE_VERSION_V2,
    },
    os::shared::process::capture,
};
use serde::Deserialize;

/// The read-only inventory of `probe.ps1`; the self-setup reads the same
/// facts (edition, policy, state) before it asks for elevation.
#[derive(Deserialize)]
pub(super) struct Inventory {
    pub(super) supported: bool,
    pub(super) home: bool,
    server: bool,
    edition: String,
    sku: u32,
    version: String,
    pub(super) enabled: bool,
    running: bool,
    port: Option<u16>,
    tls: Option<bool>,
    nla: Option<bool>,
    certificate_sha256: Option<CertificateFingerprint>,
    certificate_source: Option<String>,
    #[serde(default)]
    certificate_sha1: Option<CertificateSha1>,
    #[serde(default)]
    termservice_pid: Option<u32>,
    #[serde(default)]
    pub(super) policy_denies: bool,
    remote_apps: Vec<PublishedApp>,
    notes: Vec<String>,
    /// E29 / O10: the firewall facts `probe.ps1` read for the RDP port. A fact
    /// that was not read is `None`; an older script sends none of them.
    #[serde(default)]
    firewall_profiles: Option<Vec<FirewallProfile>>,
    #[serde(default)]
    firewall_active: Option<Vec<String>>,
    #[serde(default)]
    firewall_candidates: Option<Vec<FirewallCandidate>>,
    #[serde(default)]
    firewall_note: Option<String>,
}

pub(super) fn inventory() -> Result<Inventory, String> {
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
}

/// One firewall profile of `Get-NetFirewallProfile` in the ActiveStore
/// (`Domain`, `Private`, `Public`); `enabled` is `None` when its state was not
/// read. `inbound` is its `DefaultInboundAction` (`allow`, `block`), `None` when
/// not read: an enabled profile that allows by default admits every port.
#[derive(Deserialize, Debug, PartialEq, Eq)]
pub(super) struct FirewallProfile {
    name: String,
    enabled: Option<bool>,
    #[serde(default)]
    inbound: Option<String>,
}

impl FirewallProfile {
    /// Every fact of the profile that decides an admission was read.
    fn read(&self) -> bool {
        match self.enabled {
            None => false,
            Some(false) => true,
            Some(true) => matches!(self.inbound.as_deref(), Some("allow" | "block")),
        }
    }
}

/// One inbound allow rule that names the RDP port over TCP or any protocol.
/// `profile` is the rule's profile list as the firewall prints it (`Any`,
/// `Domain, Private`); `network` is set when one remote address is not loopback;
/// `scoped` is set for a program or service filter other than `Any`. No rule
/// name leaves the host.
#[derive(Deserialize, Debug, PartialEq, Eq)]
pub(super) struct FirewallCandidate {
    profile: String,
    network: bool,
    scoped: bool,
}

/// The reading of the RDP port by the firewall (E29, O10). `open` is `Some(true)`
/// when a proved path admits the port from a network, `Some(false)` only when
/// every fact was read and none does, and `None` when a fact is missing and no
/// path is proved. `reason` names the proof or the missing fact; it names no rule.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct FirewallReading {
    open: Option<bool>,
    reason: String,
}

/// Decides the reading from the facts `probe.ps1` read. Doubt never reads as
/// closed: without a proved network profile every profile and every rule may
/// apply, and a program or service filter counts as admitting.
pub(super) fn firewall_reading(
    profiles: Option<&[FirewallProfile]>,
    active: Option<&[String]>,
    candidates: Option<&[FirewallCandidate]>,
    note: Option<&str>,
) -> FirewallReading {
    let active_names: &[String] = active.unwrap_or(&[]);
    let all_apply = active_names.is_empty();
    let applies = |profile: &str| {
        all_apply
            || profile.split(',').map(str::trim).any(|name| {
                !matches!(name, "Domain" | "Private" | "Public")
                    || active_names.iter().any(|known| known.as_str() == name)
            })
    };
    let mut proofs: Vec<String> = Vec::new();
    for profile in profiles.unwrap_or(&[]) {
        if !applies(profile.name.as_str()) {
            continue;
        }
        if profile.enabled == Some(false) {
            proofs.push(format!(
                "das Firewallprofil {} ist ausgeschaltet und lässt jeden Port zu",
                profile.name
            ));
        } else if profile.inbound.as_deref() == Some("allow") {
            proofs.push(format!(
                "das Firewallprofil {} lässt eingehende Verbindungen ohne Regel zu",
                profile.name
            ));
        }
    }
    let rules: Vec<&FirewallCandidate> = candidates
        .unwrap_or(&[])
        .iter()
        .filter(|candidate| candidate.network && applies(candidate.profile.as_str()))
        .collect();
    if !rules.is_empty() {
        let mut text = format!(
            "{} aktive eingehende Erlaubnisregel(n) für das Netzprofil lassen den Port aus dem Netz zu",
            rules.len()
        );
        if rules.iter().any(|candidate| candidate.scoped) {
            text.push_str("; ein Programm- oder Dienstfilter zählt im Zweifel als erreichbar");
        }
        proofs.push(text);
    }
    if !proofs.is_empty() {
        return FirewallReading {
            open: Some(true),
            reason: proofs.join("; "),
        };
    }
    let unread = profiles.is_none()
        || active.is_none()
        || candidates.is_none()
        || profiles
            .unwrap_or(&[])
            .iter()
            .any(|profile| !profile.read());
    if unread {
        FirewallReading {
            open: None,
            reason: note.unwrap_or("eine Firewallangabe fehlt").to_string(),
        }
    } else {
        FirewallReading {
            open: Some(false),
            reason: "keine aktive eingehende Erlaubnisregel und kein ausgeschaltetes oder standardmäßig erlaubendes Firewallprofil lässt den Port aus dem Netz zu".into(),
        }
    }
}

/// Evidence and one diagnostics line of the firewall reading (E29, O10). The
/// lines name the port and the proof or the missing fact, never a rule name and
/// no address.
pub(super) fn firewall_lines(port: Option<u16>, reading: &FirewallReading) -> (String, String) {
    let Some(port) = port else {
        return (
            "Windows-Firewall: kein RDP-Listener belegt; der Firewallstand wird nicht bewertet"
                .into(),
            "Firewallstand des RDP-Ports nicht bewertet: kein belegter RDP-Listener".into(),
        );
    };
    let reason = &reading.reason;
    match reading.open {
        Some(true) => (
            format!("Windows-Firewall: TCP-Port {port} aus dem Netz zugelassen: {reason}; se öffnet keine Regel"),
            format!("Remote Desktop dieses Geräts ist über TCP-Port {port} aus dem Netz erreichbar: {reason}. se hat keine Regel geöffnet und ändert keine Administratorregel."),
        ),
        Some(false) => (
            format!("Windows-Firewall: kein Weg lässt TCP-Port {port} aus dem Netz zu; alle Firewallangaben sind gelesen"),
            format!("Remote Desktop dieses Geräts ist über TCP-Port {port} aus dem Netz nicht erreichbar: alle Firewallangaben sind gelesen und keine Erlaubnis gefunden."),
        ),
        None => (
            format!("Windows-Firewall: Stand für TCP-Port {port} nicht vollständig lesbar: {reason}"),
            format!("Firewallstand des RDP-Ports {port} nicht belegt: {reason}"),
        ),
    }
}

pub fn probe() -> HostProbe {
    let result = inventory();
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
    let unavailable_reason = if inventory.policy_denies {
        Some("Eine Gruppenrichtlinie verbietet Remote Desktop auf diesem Gerät".into())
    } else if !inventory.enabled {
        Some(
            "Windows Remote Desktop is off; se switches it on when a device may steer this screen"
                .into(),
        )
    } else if !inventory.running || inventory.port.is_none() {
        Some("No TermService-owned TCP listener was proved; required inventory permissions may be missing".into())
    } else {
        None
    };
    let listener_pid = inventory
        .termservice_pid
        .filter(|_| inventory.port.is_some());
    let reading = firewall_reading(
        inventory.firewall_profiles.as_deref(),
        inventory.firewall_active.as_deref(),
        inventory.firewall_candidates.as_deref(),
        inventory.firewall_note.as_deref(),
    );
    let (firewall_evidence, firewall_diagnostic) = firewall_lines(inventory.port, &reading);
    let service = HostService {
        id: "windows-rdp".into(),
        kind: if inventory.server {
            HostKind::WindowsServer
        } else {
            HostKind::WindowsWorkstation
        },
        version: Some(inventory.version),
        session_mode: if inventory.server {
            SessionMode::WindowsServerSession
        } else {
            SessionMode::WindowsConsoleLocked
        },
        port: inventory.port,
        running: inventory.running,
        tls: inventory.tls,
        nla: inventory.nla,
        certificate_sha256: inventory.certificate_sha256,
        certificate_source: inventory.certificate_source,
        remote_apps: inventory.remote_apps,
        unavailable_reason,
        certificate_sha1: inventory.certificate_sha1,
        managed_credentials: false,
        listener_pid,
        evidence: vec![
            format!(
                "Win32_OperatingSystem: {} SKU={} server={}",
                inventory.edition, inventory.sku, inventory.server
            ),
            "Read-only Terminal Server settings, TermService socket ownership and Win32_TSGeneralSetting; TLS/NLA/firewall unchanged".into(),
            firewall_evidence,
        ],
    };
    let mut report = HostProbe {
        version: RDP_PROBE_VERSION_V2,
        platform: "windows".into(),
        services: vec![service],
        unavailable_reason: None,
        diagnostics: inventory.notes,
    };
    report.diagnostics.push(firewall_diagnostic);
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

#[cfg(test)]
#[path = "rdp_netz_task_tests.rs"]
mod rdp_netz_task_tests;
