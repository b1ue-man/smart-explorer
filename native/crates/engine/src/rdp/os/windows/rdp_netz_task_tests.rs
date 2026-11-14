//! E29 / O10 (M11): the Windows probe reads the firewall for the RDP port and
//! admits a port only with proof. These tests evaluate fixed facts and fixed
//! inventory text; the facts themselves are read by `probe.ps1` on Windows only.
use super::{firewall_lines, firewall_reading, FirewallCandidate, FirewallProfile, Inventory};

const WITH_FIREWALL_FACTS: &str = r#"{"supported":true,"home":false,"server":false,"edition":"Windows 11 Pro","sku":48,"version":"10.0.26100","enabled":true,"running":true,"port":3389,"tls":true,"nla":true,"certificate_sha256":null,"certificate_source":null,"certificate_sha1":null,"termservice_pid":1234,"policy_denies":false,"remote_apps":[],"notes":[],"firewall_profiles":[{"name":"Private","enabled":false}],"firewall_active":["Private"],"firewall_candidates":[{"profile":"Any","network":true,"scoped":false}],"firewall_note":null}"#;

const WITHOUT_FIREWALL_FACTS: &str = r#"{"supported":true,"home":false,"server":false,"edition":"Windows 11 Pro","sku":48,"version":"10.0.26100","enabled":true,"running":true,"port":3389,"tls":true,"nla":true,"certificate_sha256":null,"certificate_source":null,"certificate_sha1":null,"termservice_pid":1234,"policy_denies":false,"remote_apps":[],"notes":[]}"#;

/// A profile whose default inbound action blocks (Windows default).
fn profile(name: &str, enabled: Option<bool>) -> FirewallProfile {
    FirewallProfile {
        name: name.into(),
        enabled,
        inbound: Some("block".into()),
    }
}

fn candidate(profile: &str, network: bool, scoped: bool) -> FirewallCandidate {
    FirewallCandidate {
        profile: profile.into(),
        network,
        scoped,
    }
}

fn active(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn all_enabled() -> Vec<FirewallProfile> {
    vec![
        profile("Domain", Some(true)),
        profile("Private", Some(true)),
        profile("Public", Some(true)),
    ]
}

/// An enabled inbound allow rule from the network for the active profile:
/// evidence and diagnostics both say the port is reachable, and that se opened
/// nothing.
#[test]
fn rdp_netz_task_firewall_open_rule_is_reported_in_evidence_and_diagnostics() {
    let profiles = all_enabled();
    let network = active(&["Private"]);
    let rules = vec![candidate("Domain, Private", true, false)];
    let reading = firewall_reading(
        Some(profiles.as_slice()),
        Some(network.as_slice()),
        Some(rules.as_slice()),
        None,
    );
    assert_eq!(reading.open, Some(true));
    let (evidence, diagnostic) = firewall_lines(Some(3389), &reading);
    assert!(evidence.contains("aus dem Netz zugelassen"), "{evidence}");
    assert!(evidence.contains("se öffnet keine Regel"), "{evidence}");
    assert!(
        diagnostic.contains("über TCP-Port 3389 aus dem Netz erreichbar"),
        "{diagnostic}"
    );
}

/// All facts read and no enabled inbound allow rule admits the port: the reading
/// says so, and only then.
#[test]
fn rdp_netz_task_firewall_without_network_rule_is_reported() {
    let profiles = all_enabled();
    let network = active(&["Private"]);
    let rules: Vec<FirewallCandidate> = Vec::new();
    let reading = firewall_reading(
        Some(profiles.as_slice()),
        Some(network.as_slice()),
        Some(rules.as_slice()),
        None,
    );
    assert_eq!(reading.open, Some(false));
    let (evidence, diagnostic) = firewall_lines(Some(3389), &reading);
    assert!(
        evidence.contains("kein Weg lässt TCP-Port 3389 aus dem Netz zu"),
        "{evidence}"
    );
    assert!(
        diagnostic.contains("aus dem Netz nicht erreichbar"),
        "{diagnostic}"
    );
}

/// An unreadable firewall is not reported as open or closed; no listener means
/// the rules are not evaluated at all.
#[test]
fn rdp_netz_task_firewall_unreadable_and_missing_listener_are_not_guessed() {
    let reading = firewall_reading(None, None, None, Some("Zugriff verweigert"));
    assert_eq!(reading.open, None);
    let (evidence, diagnostic) = firewall_lines(Some(3389), &reading);
    assert!(
        evidence.contains("nicht vollständig lesbar: Zugriff verweigert"),
        "{evidence}"
    );
    assert!(
        diagnostic.contains("nicht belegt: Zugriff verweigert"),
        "{diagnostic}"
    );
    let (evidence, diagnostic) = firewall_lines(None, &reading);
    assert!(evidence.contains("kein RDP-Listener belegt"), "{evidence}");
    assert!(
        diagnostic.contains("kein belegter RDP-Listener"),
        "{diagnostic}"
    );
}

/// The inventory carries the firewall facts; an inventory without them (the
/// script before this change) reads as "not read" for every fact.
#[test]
fn rdp_netz_task_inventory_reads_firewall_fields_and_older_output() {
    let with_facts: Inventory =
        serde_json::from_str(WITH_FIREWALL_FACTS).expect("inventory with firewall facts");
    assert_eq!(with_facts.firewall_profiles.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        with_facts.firewall_active.as_deref(),
        Some(active(&["Private"]).as_slice())
    );
    assert_eq!(
        with_facts.firewall_candidates.as_ref().map(Vec::len),
        Some(1)
    );
    assert_eq!(with_facts.firewall_note, None);
    let without: Inventory =
        serde_json::from_str(WITHOUT_FIREWALL_FACTS).expect("inventory without firewall facts");
    assert!(without.firewall_profiles.is_none());
    assert!(without.firewall_active.is_none());
    assert!(without.firewall_candidates.is_none());
    assert_eq!(without.firewall_note, None);
}

/// A switched-off firewall profile admits every port; the reason names the
/// profile and no rule. A switched-off profile that is not the active one does not.
#[test]
fn rdp_netz_task_firewall_disabled_profile_admits_the_port() {
    let profiles = vec![
        profile("Domain", Some(true)),
        profile("Private", Some(false)),
        profile("Public", Some(true)),
    ];
    let network = active(&["Private"]);
    let rules: Vec<FirewallCandidate> = Vec::new();
    let reading = firewall_reading(
        Some(profiles.as_slice()),
        Some(network.as_slice()),
        Some(rules.as_slice()),
        None,
    );
    assert_eq!(reading.open, Some(true));
    assert!(
        reading
            .reason
            .contains("Firewallprofil Private ist ausgeschaltet"),
        "{}",
        reading.reason
    );
    let other = vec![profile("Public", Some(false))];
    let not_active = firewall_reading(
        Some(other.as_slice()),
        Some(network.as_slice()),
        Some(rules.as_slice()),
        None,
    );
    assert_eq!(not_active.open, Some(false));
}

/// An allow rule for a profile that is not the active network profile does not
/// admit the port.
#[test]
fn rdp_netz_task_firewall_rule_for_another_profile_does_not_admit() {
    let profiles = all_enabled();
    let network = active(&["Private"]);
    let rules = vec![candidate("Public", true, false)];
    let reading = firewall_reading(
        Some(profiles.as_slice()),
        Some(network.as_slice()),
        Some(rules.as_slice()),
        None,
    );
    assert_eq!(reading.open, Some(false));
}

/// A rule with a program or service filter other than `Any` counts as open: the
/// doubt is named, not hidden.
#[test]
fn rdp_netz_task_firewall_program_filter_rule_counts_as_open() {
    let profiles = all_enabled();
    let network = active(&["Private"]);
    let rules = vec![candidate("Any", true, true)];
    let reading = firewall_reading(
        Some(profiles.as_slice()),
        Some(network.as_slice()),
        Some(rules.as_slice()),
        None,
    );
    assert_eq!(reading.open, Some(true));
    assert!(
        reading.reason.contains("Programm- oder Dienstfilter"),
        "{}",
        reading.reason
    );
}

/// Without a proved active network profile every profile and every rule may
/// apply; with no admitting fact and no proof of the network the reading is open.
#[test]
fn rdp_netz_task_firewall_unknown_network_profile_applies_every_rule() {
    let profiles = all_enabled();
    let rules = vec![candidate("Public", true, false)];
    let reading = firewall_reading(
        Some(profiles.as_slice()),
        None,
        Some(rules.as_slice()),
        None,
    );
    assert_eq!(reading.open, Some(true));
    let none: Vec<FirewallCandidate> = Vec::new();
    let unproved = firewall_reading(Some(profiles.as_slice()), None, Some(none.as_slice()), None);
    assert_eq!(unproved.open, None);
}

/// A rule that admits only loopback addresses is no network path.
#[test]
fn rdp_netz_task_firewall_loopback_only_rule_is_not_network_reach() {
    let profiles = all_enabled();
    let network = active(&["Private"]);
    let rules = vec![candidate("Any", false, false)];
    let reading = firewall_reading(
        Some(profiles.as_slice()),
        Some(network.as_slice()),
        Some(rules.as_slice()),
        None,
    );
    assert_eq!(reading.open, Some(false));
}

/// An enabled profile of the active network that allows inbound connections by
/// default admits the port without any rule; an enabled profile whose default
/// was not read is never read as closed.
#[test]
fn rdp_netz_task_firewall_default_allow_profile_admits_the_port() {
    let network = active(&["Private"]);
    let rules: Vec<FirewallCandidate> = Vec::new();
    let allowing: FirewallProfile =
        serde_json::from_str(r#"{"name":"Private","enabled":true,"inbound":"allow"}"#)
            .expect("profile row");
    let profiles = vec![
        profile("Domain", Some(true)),
        allowing,
        profile("Public", Some(true)),
    ];
    let reading = firewall_reading(
        Some(profiles.as_slice()),
        Some(network.as_slice()),
        Some(rules.as_slice()),
        None,
    );
    assert_eq!(reading.open, Some(true));
    assert!(
        reading
            .reason
            .contains("Private lässt eingehende Verbindungen ohne Regel zu"),
        "{}",
        reading.reason
    );
    let unread: FirewallProfile =
        serde_json::from_str(r#"{"name":"Private","enabled":true,"inbound":null}"#)
            .expect("profile row");
    let profiles = vec![
        profile("Domain", Some(true)),
        unread,
        profile("Public", Some(true)),
    ];
    let reading = firewall_reading(
        Some(profiles.as_slice()),
        Some(network.as_slice()),
        Some(rules.as_slice()),
        None,
    );
    assert_eq!(reading.open, None);
}
