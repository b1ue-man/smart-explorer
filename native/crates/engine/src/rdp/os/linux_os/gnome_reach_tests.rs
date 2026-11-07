//! Review r-exec-1 (batch 2026-11-07): the probe notes that a GNOME desktop
//! sharing set up by se listens on every interface, and only then.
use super::{note_network_reach, service, NETWORK_REACH_EVIDENCE, NETWORK_REACH_NOTE};
use crate::rdp::core::{
    HostKind, HostProbe, HostService, SessionMode, RDP_PROBE_VERSION, RDP_PROBE_VERSION_V2,
};

fn sharing(
    id: &str,
    kind: HostKind,
    mode: SessionMode,
    managed: bool,
    port: Option<u16>,
    running: bool,
) -> HostService {
    let mut found = service(id, kind, mode);
    found.managed_credentials = managed;
    found.port = port;
    found.running = running;
    found
}

fn managed_gnome(id: &str, mode: SessionMode) -> HostService {
    sharing(id, HostKind::Gnome, mode, true, Some(3389), true)
}

fn report(services: Vec<HostService>) -> HostProbe {
    HostProbe {
        version: RDP_PROBE_VERSION_V2,
        platform: "linux".into(),
        services,
        unavailable_reason: None,
        diagnostics: Vec::new(),
    }
}

fn occurrences(lines: &[String], text: &str) -> usize {
    lines.iter().filter(|line| line.as_str() == text).count()
}

/// One test with the cases of T-r-exec-1-a to -d: the note appears once per
/// service and once in the diagnostics, not for a sharing the person set up
/// themselves, without a proved listener or for another service kind, and it
/// is valid in the v1 form an old `RdpProbe` is answered with.
#[test]
fn exec_task_gnome_network_reach_note_marks_only_managed_listening_sharing() {
    // (a) one managed, running GNOME sharing with a port.
    let mut one = report(vec![managed_gnome(
        "gnome-console",
        SessionMode::SharedConsole,
    )]);
    note_network_reach(&mut one);
    assert_eq!(
        occurrences(&one.services[0].evidence, NETWORK_REACH_EVIDENCE),
        1
    );
    assert_eq!(occurrences(&one.diagnostics, NETWORK_REACH_NOTE), 1);
    one.validate().expect("the noted probe stays valid");

    // (b) a second call adds nothing; two such services get one text each
    // and the diagnostics line stays single.
    note_network_reach(&mut one);
    assert_eq!(
        occurrences(&one.services[0].evidence, NETWORK_REACH_EVIDENCE),
        1
    );
    assert_eq!(occurrences(&one.diagnostics, NETWORK_REACH_NOTE), 1);
    let mut two = report(vec![
        managed_gnome("gnome-console", SessionMode::SharedConsole),
        managed_gnome("gnome-headless", SessionMode::HeadlessUserSession),
    ]);
    note_network_reach(&mut two);
    note_network_reach(&mut two);
    for found in &two.services {
        assert_eq!(occurrences(&found.evidence, NETWORK_REACH_EVIDENCE), 1);
    }
    assert_eq!(occurrences(&two.diagnostics, NETWORK_REACH_NOTE), 1);
    two.validate().expect("two noted services stay valid");

    // (c) no text without credentials set up by se, without a port, without
    // a running listener, or for xrdp (its own loopback binding).
    let counter_cases = [
        (
            "credentials of the person",
            sharing(
                "gnome-console",
                HostKind::Gnome,
                SessionMode::SharedConsole,
                false,
                Some(3389),
                true,
            ),
        ),
        (
            "no port",
            sharing(
                "gnome-console",
                HostKind::Gnome,
                SessionMode::SharedConsole,
                true,
                None,
                true,
            ),
        ),
        (
            "not running",
            sharing(
                "gnome-console",
                HostKind::Gnome,
                SessionMode::SharedConsole,
                true,
                Some(3389),
                false,
            ),
        ),
        (
            "xrdp",
            sharing(
                "xrdp",
                HostKind::Xrdp,
                SessionMode::SeparateXorg,
                false,
                Some(3389),
                true,
            ),
        ),
        (
            "xrdp with managed credentials",
            sharing(
                "xrdp",
                HostKind::Xrdp,
                SessionMode::SeparateXorg,
                true,
                Some(3389),
                true,
            ),
        ),
    ];
    for (case, found) in counter_cases {
        let mut probe = report(vec![found]);
        note_network_reach(&mut probe);
        assert!(probe.services[0].evidence.is_empty(), "{case}");
        assert!(probe.diagnostics.is_empty(), "{case}");
    }

    // (d) the exact v1 form drops `managed_credentials`, keeps the texts
    // (plain strings, allowed in v1) and validates.
    assert_ne!(one.version, RDP_PROBE_VERSION);
    let old = one.into_v1();
    old.validate().expect("valid v1 probe");
    assert_eq!(old.version, RDP_PROBE_VERSION);
    assert!(!old.services[0].managed_credentials);
    assert_eq!(
        occurrences(&old.services[0].evidence, NETWORK_REACH_EVIDENCE),
        1
    );
    assert_eq!(occurrences(&old.diagnostics, NETWORK_REACH_NOTE), 1);
}
