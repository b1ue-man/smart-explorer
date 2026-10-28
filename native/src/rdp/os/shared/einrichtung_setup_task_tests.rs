//! Task suite of work package WP1 „Host“, RDP self-setup (plan 2026-10-28,
//! design 3, 2.3, 9.2, 10.1): trigger gating, setup states, probe v2, the
//! service rank and the pure decisions of the platform adapters; H3 (xrdp
//! loopback as root) and H4 (Windows, elevated) change the host and run only
//! with `--ignored` in the suite. Prefix `einrichtung_task_`.
use super::*;
use crate::rdp::core::{
    CertificateSha1, HostKind, HostProbe, HostService, SessionMode, RDP_PROBE_VERSION,
    RDP_PROBE_VERSION_V2,
};

fn marker(state: SetupState) -> Marker {
    Marker {
        state,
        trigger: SetupTrigger::VersionStart,
        at: 1,
    }
}

#[test]
fn einrichtung_task_triggers_follow_the_marker_rules() {
    use SetupTrigger::{Grant, PeerOnline, PeerRequest, VersionStart};
    for trigger in [Grant, VersionStart, PeerOnline, PeerRequest] {
        let first = decide(trigger, None);
        assert_eq!(first, Decision::Run, "{trigger:?}");
    }
    let declined = marker(SetupState::Declined);
    assert_eq!(decide(Grant, Some(&declined)), Decision::Run);
    assert_eq!(decide(PeerRequest, Some(&declined)), Decision::Run);
    let keep = Decision::Keep(SetupState::Declined);
    assert_eq!(decide(VersionStart, Some(&declined)), keep);
    assert_eq!(decide(PeerOnline, Some(&declined)), keep);
    for retry in ["no_interactive_session", "keyring_locked"] {
        let failed = marker(SetupState::Failed(retry.into()));
        assert_eq!(decide(PeerOnline, Some(&failed)), Decision::Run, "{retry}");
        let kept = decide(VersionStart, Some(&failed));
        assert!(matches!(kept, Decision::Keep(_)), "{retry}");
    }
    let package = marker(SetupState::Failed("package:100:E".into()));
    let kept = decide(PeerOnline, Some(&package));
    assert!(matches!(kept, Decision::Keep(_)));
    let impossible = marker(SetupState::Impossible("windows_home".into()));
    let kept = decide(PeerOnline, Some(&impossible));
    assert!(matches!(kept, Decision::Keep(_)));
    assert_eq!(decide(PeerRequest, Some(&impossible)), Decision::Run);
}

#[test]
fn einrichtung_task_setup_states_have_one_text_form() {
    let states = [
        SetupState::Done,
        SetupState::Declined,
        SetupState::NotNeeded,
        SetupState::Impossible("windows_home".into()),
        SetupState::Failed("package:100:E: Unable to locate package xrdp".into()),
    ];
    for state in states {
        let text = state.to_string();
        assert_eq!(SetupState::parse(&text).expect("parse"), state, "{text}");
        let json = serde_json::to_value(&state).expect("json");
        assert_eq!(json, serde_json::Value::String(text));
        assert!(!state.text().is_empty());
    }
    for invalid in ["", "failed:", "impossible", "later", "failed:a\nb"] {
        assert!(SetupState::parse(invalid).is_err(), "{invalid:?}");
    }
    assert!(!SetupState::Impossible("android".into()).setup_possible());
    assert!(SetupState::Declined.setup_possible());
    let stored = marker(SetupState::Failed("no_listener: xrdp".into()));
    let json = serde_json::to_string(&stored).expect("marker");
    let back: Marker = serde_json::from_str(&json).expect("marker");
    assert_eq!(back, stored);
}

fn service(id: &str, mode: SessionMode, startable: bool) -> HostService {
    let kind = match id {
        "xrdp" => HostKind::Xrdp,
        "krdp" => HostKind::Krdp,
        _ => HostKind::Gnome,
    };
    HostService {
        id: id.into(),
        kind,
        version: None,
        session_mode: mode,
        port: Some(3389),
        running: startable,
        tls: Some(true),
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

fn probe(services: Vec<HostService>) -> HostProbe {
    HostProbe {
        version: RDP_PROBE_VERSION_V2,
        platform: "linux".into(),
        services,
        unavailable_reason: None,
        diagnostics: Vec::new(),
    }
}

#[test]
fn einrichtung_task_service_rank_prefers_the_shared_console() {
    let mut report = probe(vec![
        service("xrdp", SessionMode::SeparateXorg, true),
        service("gnome-headless", SessionMode::HeadlessUserSession, true),
        service("gnome-console", SessionMode::SharedConsole, false),
    ]);
    report.validate().expect("valid inventory");
    let preferred = report.preferred_service().expect("startable service");
    // A non-startable service of rank 1 never wins.
    assert_eq!(preferred.id, "gnome-headless");
    report.services[2].running = true;
    let preferred = report.preferred_service().expect("startable service");
    assert_eq!(preferred.id, "gnome-console");
    let tie = probe(vec![
        service("krdp", SessionMode::SharedConsole, true),
        service("gnome-console", SessionMode::SharedConsole, true),
    ]);
    let preferred = tie.preferred_service().expect("startable service");
    // A tie goes to the smaller service ID.
    assert_eq!(preferred.id, "gnome-console");
    let (shared, exclusive) = (SessionMode::SharedConsole, SessionMode::ExclusiveConsole);
    assert!(shared.rank() < exclusive.rank());
    let (xorg, xvnc) = (SessionMode::SeparateXorg, SessionMode::SeparateXvnc);
    assert!(xorg.rank() < xvnc.rank());
}

#[test]
fn einrichtung_task_probe_v2_fields_and_the_exact_v1_form() {
    let sha1 = CertificateSha1::parse(&"ab:".repeat(19)).err();
    assert!(sha1.is_some(), "19 bytes are no SHA-1");
    let sha1 = CertificateSha1::parse(&"aB".repeat(20)).expect("thumbprint");
    assert_eq!(sha1.as_str(), "AB".repeat(20));
    assert_eq!(sha1.bytes(), [0xAB; 20]);
    let mut gnome = service("gnome-console", SessionMode::SharedConsole, true);
    gnome.certificate_sha1 = Some(sha1);
    gnome.managed_credentials = true;
    gnome.listener_pid = Some(77);
    let report = probe(vec![gnome]);
    report.validate().expect("v2 inventory");
    let json = serde_json::to_value(&report).expect("json");
    let first = &json["services"][0];
    assert_eq!(first["managed_credentials"], serde_json::json!(true));
    // The listener PID stays on the host.
    assert!(first.get("listener_pid").is_none());
    let v1 = report.clone().into_v1();
    assert_eq!(v1.version, RDP_PROBE_VERSION);
    v1.validate().expect("v1 inventory");
    let json = serde_json::to_value(&v1).expect("json");
    assert!(json["services"][0].get("certificate_sha1").is_none());
    assert!(json["services"][0].get("managed_credentials").is_none());
    let mut mixed = v1;
    mixed.services[0].managed_credentials = true;
    assert!(mixed.validate().is_err(), "a v1 probe carries no v2 field");
}

#[cfg(windows)]
#[test]
fn einrichtung_task_windows_setup_inputs_and_exit_codes() {
    use super::platform::{state_for_exit, valid_sid};
    assert!(valid_sid("S-1-5-21-111-222-333-1001"));
    for invalid in ["", "S-1-5-21-1'; whoami", "S-1-", "S-1-5--1", "X-1-5-18"] {
        assert!(!valid_sid(invalid), "{invalid:?}");
    }
    assert_eq!(state_for_exit(0), SetupState::Done);
    assert_eq!(state_for_exit(1223), SetupState::Declined);
    let policy = SetupState::Impossible("policy_denies_rdp".into());
    assert_eq!(state_for_exit(10), policy);
    let home = SetupState::Impossible("windows_home".into());
    assert_eq!(state_for_exit(11), home);
    assert!(matches!(state_for_exit(20), SetupState::Failed(_)));
    assert!(matches!(state_for_exit(1224), SetupState::Failed(_)));
}

#[cfg(target_os = "linux")]
#[test]
fn einrichtung_task_xrdp_listens_on_loopback_only() {
    use super::platform::{loopback_config, parse_port};
    assert_eq!(parse_port("3389"), Some((3389, false)));
    assert_eq!(parse_port("tcp://:3390"), Some((3390, false)));
    assert_eq!(parse_port("tcp://.:3389"), Some((3389, true)));
    assert_eq!(parse_port("tcp://127.0.0.1:3391"), Some((3391, true)));
    assert_eq!(parse_port("tcp6://{::1}:3389"), Some((3389, true)));
    assert_eq!(parse_port("vsock://-1:3389"), None);
    let packaged = "[Globals]\nport=3389\nsecurity_layer=negotiate\n\n[Xorg]\nname=Xorg\n";
    let (rewritten, changed) = loopback_config(packaged, true).expect("se installed xrdp");
    assert!(changed);
    assert!(rewritten.contains("port=tcp://.:3389\n"));
    assert!(rewritten.contains("security_layer=negotiate\n"));
    assert!(rewritten.contains("[Xorg]\nname=Xorg\n"));
    let (again, changed) = loopback_config(&rewritten, true).expect("second run");
    assert!(!changed, "a loopback configuration stays as it is");
    assert_eq!(again, rewritten);
    let foreign = loopback_config(packaged, false).expect_err("foreign network-wide xrdp");
    assert_eq!(foreign, "xrdp_network_wide_not_se_installed");
    let foreign_loopback = "[Globals]\nport=tcp://.:3389\nsecurity_layer=rdp\n";
    let (fixed, changed) = loopback_config(foreign_loopback, false).expect("loopback xrdp");
    assert!(changed && fixed.contains("security_layer=tls\n"));
    let sparse = "[Globals]\nautorun=\n[Xvnc]\nlib=libvnc.so\n";
    let (filled, _) = loopback_config(sparse, true).expect("missing keys");
    let globals = filled.split("[Xvnc]").next().expect("globals");
    assert!(globals.contains("port=tcp://.:3389"));
    assert!(globals.contains("security_layer=tls"));
    // A file without [Globals] is refused.
    assert!(loopback_config("port=3389\n", true).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn einrichtung_task_linux_setup_decisions() {
    use super::platform::{
        credential_input, desktop_from, has_users, managed_username, package_manager_from,
        pkexec_outcome, Desktop, PackageManager, SystemStep,
    };
    let ubuntu = "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n";
    assert_eq!(package_manager_from(ubuntu), Some(PackageManager::Apt));
    let fedora = "ID=fedora\nVERSION_ID=41\n";
    assert_eq!(package_manager_from(fedora), Some(PackageManager::Dnf));
    let alma = "ID=\"almalinux\"\nID_LIKE=\"rhel centos fedora\"\n";
    assert_eq!(package_manager_from(alma), Some(PackageManager::Dnf));
    assert_eq!(package_manager_from("ID=arch\n"), None);
    assert_eq!(desktop_from("ubuntu:GNOME"), Desktop::Gnome);
    assert_eq!(desktop_from("KDE"), Desktop::Plasma);
    assert_eq!(desktop_from("XFCE"), Desktop::Other);
    let steps = ["install-xrdp", "configure-xrdp", "install-gnome-rd"];
    for name in steps.into_iter().chain(["install-krdp"]) {
        let step = SystemStep::parse(name).expect("fixed step");
        assert_eq!(step.name(), name);
    }
    assert!(SystemStep::parse("install-xrdp; rm -rf /").is_none());
    assert_eq!(pkexec_outcome(Some(126), "", ""), SetupState::Declined);
    let agent = "Error executing command as another user: No authentication agent found.";
    let no_agent = SetupState::Failed("no_polkit_agent".into());
    assert_eq!(pkexec_outcome(Some(127), "", agent), no_agent);
    let child = "state=impossible:package_manager_unsupported\n";
    let reported = pkexec_outcome(Some(1), child, "");
    let expected = SetupState::Impossible("package_manager_unsupported".into());
    assert_eq!(reported, expected);
    let done = pkexec_outcome(Some(0), "state=done\n", "");
    assert_eq!(done, SetupState::Done);
    let user = managed_username("device-1");
    assert!(user.starts_with("se-") && user.len() == 11);
    assert!(user[3..].bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(user, managed_username("device-1"), "fixed per device");
    assert_eq!(credential_input("se-1", "pw"), "se-1\npw\n");
    assert!(has_users("alice", "false"));
    assert!(has_users("", "true"));
    assert!(!has_users(" , ", "false"));
}

/// H3 (design 10.1, ubuntu-24.04 as root): install and configure xrdp, then
/// it listens on 127.0.0.1:3389 only and the probe finds it startable.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "installs and starts xrdp; the suite runs it as root with --ignored"]
fn einrichtung_task_h3_xrdp_on_loopback_as_root() {
    let line = run_system_step("install-xrdp").expect("install-xrdp");
    assert_eq!(line, "state=done");
    let line = run_system_step("configure-xrdp").expect("configure-xrdp");
    assert_eq!(line, "state=done");
    let ss = std::process::Command::new("ss")
        .args(["-ltnH"])
        .output()
        .expect("ss");
    let listening = String::from_utf8_lossy(&ss.stdout);
    assert!(listening.contains("127.0.0.1:3389"), "{listening}");
    assert!(!listening.contains("0.0.0.0:3389"));
    assert!(!listening.contains("[::]:3389"));
    let report = refresh_probe();
    let preferred = report.preferred_service().expect("startable service");
    assert_eq!(preferred.id, "xrdp", "{report:?}");
}

#[cfg(windows)]
fn powershell(command: &str) -> String {
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", command])
        .output()
        .expect("powershell");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// H4 (design 10.1, windows-2025, elevated): the setup switches RDP on and
/// starts TermService, leaves the „Remote Desktop“ firewall group as it was,
/// and probe v2 reports the listener's SHA-1 thumbprint.
#[cfg(windows)]
#[test]
#[ignore = "switches Remote Desktop on; the suite runs it elevated with --ignored"]
fn einrichtung_task_h4_windows_rdp_switched_on_without_firewall_change() {
    let rules = "Get-NetFirewallRule -Group '@FirewallAPI.dll,-28752' | Sort-Object Name | \
        ForEach-Object { $_.Name + '=' + $_.Enabled }";
    let before = powershell(rules);
    let state = super::platform::ensure(&refresh_probe(), Some("h4-device"));
    assert_eq!(state, SetupState::Done);
    let key = "'HKLM:\\SYSTEM\\CurrentControlSet\\Control\\Terminal Server'";
    let deny = powershell(&format!("(Get-ItemProperty {key}).fDenyTSConnections"));
    assert_eq!(deny, "0");
    assert_eq!(powershell("(Get-Service TermService).Status"), "Running");
    // The „Remote Desktop“ firewall group stays unchanged.
    assert_eq!(powershell(rules), before);
    let report = refresh_probe();
    let service = report.preferred_service().expect("startable Windows RDP");
    let wmi = powershell(
        "(Get-CimInstance -Namespace root/cimv2/TerminalServices -ClassName Win32_TSGeneralSetting \
         -Filter \"TerminalName='RDP-Tcp'\").SSLCertificateSHA1Hash",
    );
    let sha1 = service.certificate_sha1.as_ref().expect("thumbprint");
    assert_eq!(sha1.as_str(), wmi.to_ascii_uppercase());
}
