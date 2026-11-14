//! E29 (M11): the GNOME desktop sharing that se sets up is switched on for a
//! session and off after the last one; a restart removes an orphaned switch-on.
//! Clock ticks, connection rows and status text are fixed inputs; no grdctl,
//! systemctl or network call runs in these tests.
use super::super::inventory::{service, Unit};
use super::net::accepted_from_loopback;
use super::session::{own_record, owner_alive, SessionClock, SESSION_GRACE};
use super::{apply_status, rdp_enabled, rdp_setting, NETWORK_REACH_NOTE};
use crate::rdp::core::{HostKind, SessionMode};
use crate::rdp::host_setup::{switched_off_between_sessions, OFF_BETWEEN_SESSIONS};
use std::time::{Duration, Instant};

/// Two overlapping sessions (0–40 s and 20–70 s), ticks every two seconds: the
/// sharing stays on until the grace has passed after the last tick with a
/// connection (68 s), not after the first session ends.
#[test]
fn rdp_netz_task_gnome_two_sessions_release_only_after_the_last_one() {
    let start = Instant::now();
    let mut clock = SessionClock::started(start);
    for step in 0..=75u64 {
        let at = 2 * step;
        let first = (0..40u64).contains(&at);
        let second = (20..70u64).contains(&at);
        clock.observe(
            usize::from(first) + usize::from(second),
            start + Duration::from_secs(at),
        );
    }
    let last_seen = start + Duration::from_secs(68);
    assert!(!clock.due(last_seen + SESSION_GRACE - Duration::from_secs(2)));
    assert!(clock.due(last_seen + SESSION_GRACE));
}

/// One session that is switched on and then lets the clock run out (the client
/// never connected): the sharing is due after the grace, not before.
#[test]
fn rdp_netz_task_gnome_switch_on_without_a_connection_is_due_after_the_grace() {
    let start = Instant::now();
    let clock = SessionClock::started(start);
    assert!(!clock.due(start + SESSION_GRACE - Duration::from_secs(1)));
    assert!(clock.due(start + SESSION_GRACE));
}

/// A new connection during the grace moves the release on; the sharing is not
/// due at the old moment.
#[test]
fn rdp_netz_task_gnome_a_connection_during_the_grace_moves_the_release() {
    let start = Instant::now();
    let mut clock = SessionClock::started(start);
    clock.observe(1, start + Duration::from_secs(60));
    assert!(!clock.due(start + SESSION_GRACE));
    assert!(clock.due(start + Duration::from_secs(60) + SESSION_GRACE));
}

/// Only accepted connections from loopback peers on the RDP port count: the
/// se tunnel is one, a LAN peer, a listening socket and another port are not.
#[test]
fn rdp_netz_task_gnome_connection_rows_count_only_accepted_loopback_on_the_rdp_port() {
    let ports = [3389u16];
    let loopback = "   0: 0100007F:0D3D 0100007F:E1A4 01 00000000:00000000 00:00000000 00000000  1000        0 12345 1 0000000000000000 100 0 0 10 0";
    assert!(accepted_from_loopback(loopback, &ports));
    let lan = "   1: 0A00000A:0D3D 0A00000B:E1A5 01 00000000:00000000 00:00000000 00000000  1000        0 12346 1 0000000000000000 100 0 0 10 0";
    assert!(!accepted_from_loopback(lan, &ports));
    let listening = "   2: 00000000:0D3D 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12347 1 0000000000000000 100 0 0 10 0";
    assert!(!accepted_from_loopback(listening, &ports));
    let other_port = "   3: 0100007F:0050 0100007F:E1A6 01 00000000:00000000 00:00000000 00000000  1000        0 12348 1 0000000000000000 100 0 0 10 0";
    assert!(!accepted_from_loopback(other_port, &ports));
    let ipv6 = "   4: 00000000000000000000000001000000:0D3D 00000000000000000000000001000000:E1A7 01 00000000:00000000 00:00000000 00000000  1000        0 12349 1 0000000000000000 100 0 0 10 0";
    assert!(accepted_from_loopback(ipv6, &ports));
}

/// `enabled` counts only inside the `RDP:` block of `grdctl status`.
#[test]
fn rdp_netz_task_gnome_status_reads_enabled_only_in_the_rdp_block() {
    let on =
        "RDP:\n\tStatus: enabled\n\tPort: 3389\n\tNegotiate port: no\nVNC:\n\tStatus: disabled\n";
    assert!(rdp_enabled(on));
    assert_eq!(rdp_setting(on, "Port"), Some("3389"));
    let off = "RDP:\n\tStatus: disabled\n\tPort: 3389\n";
    assert!(!rdp_enabled(off));
    let other_block_only = "VNC:\n\tStatus: enabled\nRDP:\n\tStatus: disabled\n";
    assert!(!rdp_enabled(other_block_only));
}

/// Restart cleanup: a record of a living process is kept; a record of a process
/// that is gone, or whose pid was reused (other start time), marks an orphan.
#[test]
fn rdp_netz_task_gnome_orphan_check_keeps_a_living_owner_and_drops_a_gone_one() {
    let own = own_record().expect("start time of this test process");
    assert!(owner_alive(&own));
    assert!(!owner_alive("4294967295 1"));
    assert!(!owner_alive("not a record"));
    let reused = own
        .rsplit_once(' ')
        .map(|(pid, _)| format!("{pid} 0"))
        .expect("record has two fields");
    assert!(!owner_alive(&reused));
}

/// A switched-off sharing that se manages reports the session reason; any other
/// disabled sharing keeps its own reason.
#[test]
fn rdp_netz_task_gnome_switched_off_sharing_reports_the_session_reason() {
    let status = "RDP:\n\tStatus: disabled\n\tPort: 3389\n\tNegotiate port: no\n";
    let inactive = Unit {
        active: false,
        pid: 0,
        command: Vec::new(),
    };
    let mut managed = service("gnome-console", HostKind::Gnome, SessionMode::SharedConsole);
    managed.managed_credentials = true;
    apply_status(&mut managed, status, &inactive);
    assert!(!managed.running);
    assert_eq!(
        managed.unavailable_reason.as_deref(),
        Some(OFF_BETWEEN_SESSIONS)
    );
    let mut own_sharing = service("gnome-console", HostKind::Gnome, SessionMode::SharedConsole);
    apply_status(&mut own_sharing, status, &inactive);
    assert_eq!(
        own_sharing.unavailable_reason.as_deref(),
        Some("GNOME RDP backend is disabled or its status is unreadable")
    );
}

/// The person's note says the reachability holds only during a session.
#[test]
fn rdp_netz_task_gnome_reach_note_names_the_session_only() {
    assert!(NETWORK_REACH_NOTE.contains("nur während einer Sitzung"));
}

/// The case of a running GNOME user unit: `running` is true while the sharing is
/// switched off, so the condition must not depend on `running` (O1, M11).
#[test]
fn rdp_netz_task_gnome_switched_off_with_an_active_unit_is_the_off_state() {
    let status = "RDP:\n\tStatus: disabled\n\tPort: 3389\n\tNegotiate port: no\n";
    let active = Unit {
        active: true,
        pid: 0,
        command: Vec::new(),
    };
    let mut managed = service("gnome-console", HostKind::Gnome, SessionMode::SharedConsole);
    managed.managed_credentials = true;
    apply_status(&mut managed, status, &active);
    assert!(managed.running);
    assert_eq!(
        managed.unavailable_reason.as_deref(),
        Some(OFF_BETWEEN_SESSIONS)
    );
    assert!(switched_off_between_sessions(&managed));
}
