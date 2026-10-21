//! Extra SSH connections and the choice of a channel (E13, security answer
//! E13-S15 and S13): who may open one, when the next one is worth it, the
//! block after a failure, the host-key rule, the log line, and that a range
//! never ends up without a channel only because none is free. No network:
//! the decisions are pure.
use super::config::{PasswordSource, SftpAuth, SftpLoginOrigin};
use super::host_key::{HostKeyVerdict, UnknownHostKey};
use super::link_log::{self, LinkIdent};
use super::link_rules::{
    account_key, begin_login, hint_text, limit_after_refusal, plan, same_host_key, seat,
    server_key, Blocks, Candidate, Growth, Logins, Seat, Step, BLOCK_FOR, SETTLE,
};
use super::pool_channel::handle_cap_of;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MB: f64 = 1_000_000.0;

fn candidate(load: usize, cap: usize) -> Candidate {
    Candidate { load, cap }
}

fn main_key() -> UnknownHostKey {
    UnknownHostKey {
        host: "nas.example".into(),
        port: 22,
        algorithm: "ssh-ed25519".into(),
        fingerprint: "SHA256:main".into(),
    }
}

fn stored_password() -> SftpLoginOrigin {
    let source: PasswordSource = Arc::new(|| Ok(zeroize::Zeroizing::new("secret".to_string())));
    SftpLoginOrigin::StoredPassword(source)
}

#[test]
fn speed_task_sftp_seat_idle_channel_first_then_open_then_share() {
    assert_eq!(seat(&[candidate(0, 64)], 0, None), Seat::Use(0));
    assert_eq!(seat(&[candidate(3, 64)], 0, None), Seat::Open);
    assert_eq!(seat(&[], 0, None), Seat::Open);
    // The limit is reached (two channels): the least busy one is shared.
    let shared = [candidate(3, 64), candidate(5, 64)];
    assert_eq!(seat(&shared, 0, Some(2)), Seat::Use(0));
    // A channel in the making counts against the limit.
    assert_eq!(seat(&[candidate(3, 64)], 1, Some(2)), Seat::Use(0));
}

#[test]
fn speed_task_sftp_seat_range_never_goes_to_the_main_session_while_a_channel_exists() {
    // A limit of 0 (learned from a refusal) with a channel still in the pool:
    // the range shares it.
    assert_eq!(seat(&[candidate(1, 64)], 0, Some(0)), Seat::Use(0));
    // Only a pool that may hold no channel at all sends ranges to the main
    // session (which then serves them as a stream, main_range.rs).
    assert_eq!(seat(&[], 0, Some(0)), Seat::MainSession);
}

#[test]
fn speed_task_sftp_seat_waits_when_every_channel_holds_its_handles() {
    let full = [candidate(64, 64), candidate(64, 64)];
    assert_eq!(seat(&full, 0, Some(2)), Seat::Wait);
    // A channel whose server allows fewer handles is skipped for one with room.
    let mixed = [candidate(16, 16), candidate(20, 64)];
    assert_eq!(seat(&mixed, 0, Some(2)), Seat::Use(1));
    // The least busy channel with room, not the first.
    let spread = [candidate(9, 64), candidate(2, 64), candidate(2, 64)];
    assert_eq!(seat(&spread, 0, Some(3)), Seat::Use(1));
}

#[test]
fn speed_task_sftp_handles_per_channel_are_min_of_server_limit_and_64() {
    assert_eq!(handle_cap_of(None), 64);
    assert_eq!(handle_cap_of(Some(0)), 64);
    assert_eq!(handle_cap_of(Some(16)), 16);
    assert_eq!(handle_cap_of(Some(64)), 64);
    assert_eq!(handle_cap_of(Some(100_000)), 64);
}

#[test]
fn speed_task_sftp_refusal_counts_channels_still_in_setup() {
    // MaxSessions 10: the main session and nine pool channels fit. On a slow
    // link all nine are accepted before any setup is done; the tenth is
    // refused. Counting only the ready ones (none yet) learned a limit of 0
    // and sent every later range to the main session.
    assert_eq!(limit_after_refusal(None, 0, 9), 7);
    assert_eq!(limit_after_refusal(None, 3, 6), 7);
    // The old count of ready channels only.
    assert_eq!(limit_after_refusal(None, 3, 0), 1);
    // A smaller limit learned earlier stays.
    assert_eq!(limit_after_refusal(Some(5), 0, 9), 5);
    assert_eq!(limit_after_refusal(Some(9), 0, 9), 7);
}

#[test]
fn speed_task_sftp_growth_opens_one_at_a_time_while_throughput_rises() {
    let start = Instant::now();
    let mut growth = Growth::new();
    // No demand, or no data: nothing to open.
    assert_eq!(growth.window(start, 50.0 * MB, false), Step::Hold);
    assert_eq!(growth.window(start, 0.0, true), Step::Hold);
    // Demand with data: one connection, the rate before it is the baseline.
    assert_eq!(growth.window(start, 10.0 * MB, true), Step::Open);
    // While it is being set up nothing else opens.
    let later = start + Duration::from_secs(1);
    assert_eq!(growth.window(later, 10.0 * MB, true), Step::Hold);
    let up = start + Duration::from_secs(3);
    growth.installed(up);
    // A window that began before the settling time says nothing yet.
    let early = up + Duration::from_secs(1);
    assert_eq!(growth.window(early, 30.0 * MB, true), Step::Hold);
    // +100 %: worth another one, again one only.
    let judged = up + SETTLE;
    assert_eq!(growth.window(judged, 20.0 * MB, true), Step::Open);
    let second = judged + Duration::from_secs(2);
    growth.installed(second);
    // +3 %: below 10 %, no further connection.
    let judged = second + SETTLE;
    assert_eq!(growth.window(judged, 20.6 * MB, true), Step::Stop);
    assert!(growth.stopped());
    assert_eq!(growth.window(judged, 40.0 * MB, true), Step::Hold);
}

#[test]
fn speed_task_sftp_growth_needs_ten_percent_not_less() {
    let start = Instant::now();
    let mut growth = Growth::new();
    assert_eq!(growth.window(start, 100.0 * MB, true), Step::Open);
    growth.installed(start);
    let judged = start + SETTLE;
    // +9 %: stop.
    assert_eq!(growth.window(judged, 109.0 * MB, true), Step::Stop);
    let mut growth = Growth::new();
    assert_eq!(growth.window(start, 100.0 * MB, true), Step::Open);
    growth.installed(start);
    // +12 %: the next one, if the transfer still asks for more.
    assert_eq!(growth.window(judged, 112.0 * MB, true), Step::Open);
}

#[test]
fn speed_task_sftp_growth_failure_stops_and_a_cancelled_attempt_may_retry() {
    let start = Instant::now();
    let mut growth = Growth::new();
    assert_eq!(growth.window(start, 10.0 * MB, true), Step::Open);
    growth.failed();
    assert!(growth.stopped());
    assert_eq!(growth.window(start, 10.0 * MB, true), Step::Hold);
    // A new job (an idle window between) starts afresh.
    growth.idle();
    assert_eq!(growth.window(start, 10.0 * MB, true), Step::Open);
    // An attempt that never started (another login runs) is not a failure.
    growth.cancelled();
    assert_eq!(growth.window(start, 10.0 * MB, true), Step::Open);
}

#[test]
fn speed_task_sftp_other_host_key_means_no_extra_connection_and_a_block() {
    let trusted = HostKeyVerdict::Trusted;
    // The same key as the verified main connection: accepted.
    assert!(same_host_key(trusted.clone(), "ssh-ed25519", "SHA256:main", &main_key()).is_ok());
    // Another fingerprint, another algorithm: refused before any login.
    let swapped = same_host_key(trusted.clone(), "ssh-ed25519", "SHA256:other", &main_key());
    assert!(swapped.is_err());
    let other_type = same_host_key(trusted, "ecdsa-sha2-nistp256", "SHA256:main", &main_key());
    assert!(other_type.is_err());
    // The trust store has the last word: unknown, changed or revoked keys are
    // refused even when they equal the main connection's key.
    for verdict in [
        HostKeyVerdict::Unknown,
        HostKeyVerdict::Changed {
            source: "known_hosts_sftp.txt".into(),
        },
        HostKeyVerdict::Revoked {
            source: "known_hosts".into(),
        },
    ] {
        assert!(same_host_key(verdict, "ssh-ed25519", "SHA256:main", &main_key()).is_err());
    }
    // The refusal blocks the account for 15 minutes and ends the growth: no
    // retry loop.
    let reason = swapped.expect_err("refused");
    let now = Instant::now();
    let key = account_key("nas.example", 22, "alice");
    let mut blocks = Blocks::default();
    blocks.block(now, &key, &reason);
    let mut growth = Growth::new();
    assert_eq!(growth.window(now, 10.0 * MB, true), Step::Open);
    growth.failed();
    assert_eq!(blocks.blocked(now, &key).as_deref(), Some(reason.as_str()));
    assert_eq!(growth.window(now, 10.0 * MB, true), Step::Hold);
}

#[test]
fn speed_task_sftp_block_lasts_fifteen_minutes_per_host_port_and_user() {
    assert_eq!(BLOCK_FOR, Duration::from_secs(15 * 60));
    let now = Instant::now();
    let key = account_key("NAS.example", 22, "alice");
    assert_eq!(key, account_key("nas.example", 22, "alice"), "host case");
    let mut blocks = Blocks::default();
    blocks.block(now, &key, "Anmeldung abgelehnt");
    let almost = now + Duration::from_secs(14 * 60 + 59);
    assert_eq!(
        blocks.blocked(almost, &key).as_deref(),
        Some("Anmeldung abgelehnt")
    );
    assert_eq!(
        blocks.blocked(now, &account_key("nas.example", 22, "bob")),
        None
    );
    assert_eq!(
        blocks.blocked(now, &account_key("nas.example", 2222, "alice")),
        None
    );
    let after = now + Duration::from_secs(15 * 60 + 1);
    assert_eq!(blocks.blocked(after, &key), None, "the block ends");
}

#[test]
fn speed_task_sftp_one_unauthenticated_connection_per_server_at_a_time() {
    let mut logins = Logins::default();
    assert!(logins.begin("a.example:22"));
    assert!(!logins.begin("a.example:22"), "a second login waits");
    assert!(
        logins.begin("b.example:22"),
        "another server is independent"
    );
    logins.end("a.example:22");
    assert!(logins.begin("a.example:22"));
    // The process-wide slot releases on drop.
    let key = server_key("Speed-Task-Slot.example", 22);
    let first = begin_login(&key).expect("free");
    assert!(begin_login(&key).is_none());
    drop(first);
    assert!(begin_login(&key).is_some());
}

#[test]
fn speed_task_sftp_extra_login_only_for_stored_password_or_key_never_agent() {
    let password = SftpAuth::Password("secret".into());
    assert!(plan(&password, stored_password()).is_some());
    // A person typed something, or nothing is known: no extra connection.
    assert!(plan(&password, SftpLoginOrigin::Interactive).is_none());
    // The SSH agent never: it may ask for a confirmation or a touch.
    assert!(plan(&SftpAuth::Agent, stored_password()).is_none());
    assert!(plan(&SftpAuth::Agent, SftpLoginOrigin::StoredKey).is_none());
    assert!(plan(&SftpAuth::Agent, SftpLoginOrigin::Interactive).is_none());
    // The origin must match the method of the main login.
    assert!(plan(&password, SftpLoginOrigin::StoredKey).is_none());
    let key = SftpAuth::Key {
        path: "/nonexistent/key".into(),
        passphrase: None,
    };
    assert!(plan(&key, stored_password()).is_none());
    assert!(plan(&key, SftpLoginOrigin::Interactive).is_none());
    // A stored key that cannot be decrypted gives no extra connection.
    assert!(plan(&key, SftpLoginOrigin::StoredKey).is_none());
}

#[test]
fn speed_task_sftp_hint_names_connections_and_the_block() {
    assert_eq!(hint_text("nas", 1, None), None);
    assert_eq!(
        hint_text("nas", 3, None).as_deref(),
        Some("3 Verbindungen zu nas")
    );
    assert_eq!(
        hint_text("nas", 1, Some("Anmeldung abgelehnt")).as_deref(),
        Some("Weitere Verbindungen zu nas 15 Minuten gesperrt: Anmeldung abgelehnt")
    );
    assert_eq!(
        hint_text("nas", 2, Some("Zeitüberschreitung")).as_deref(),
        Some(
            "2 Verbindungen zu nas; Weitere Verbindungen zu nas 15 Minuten gesperrt: \
             Zeitüberschreitung"
        )
    );
}

#[test]
fn speed_task_sftp_log_line_has_the_facts_and_no_line_breaks() {
    let ident = LinkIdent {
        host: "nas.example".into(),
        port: 2222,
        user: "alice".into(),
        method: "password".into(),
    };
    let line = link_log::line("geöffnet", &ident, 3, "weitere\nVerbindung");
    for fact in [
        "geöffnet",
        "nas.example",
        "2222",
        "alice",
        "password",
        "offene Verbindungen 3",
        "weitere Verbindung",
    ] {
        assert!(line.contains(fact), "{fact} in {line}");
    }
    assert!(!line.contains('\n'));
}
