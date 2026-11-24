//! The decisions of the automatic codec installation: retry schedule,
//! background steps, the session gate, the cancel rule of a session's
//! download, the lock and the state texts. No network, no library.
use super::*;

#[test]
fn codec_runtime_task_retry_schedule_doubles_to_one_hour() {
    let delays: Vec<u64> = (1..=9).map(|n| retry_delay(n).as_secs()).collect();
    assert_eq!(delays, [30, 60, 120, 240, 480, 960, 1920, 3600, 3600]);
    assert_eq!(retry_delay(0).as_secs(), 30, "a first failure waits 30 s");
    assert_eq!(retry_delay(u32::MAX).as_secs(), 3600);

    let first = next_record(None, "no answer", 1_000);
    assert_eq!(
        first,
        InstallRecord {
            failures: 1,
            failed_at: 1_000,
            reason: "no answer".into(),
            retry_at: 1_030,
        }
    );
    let third = next_record(Some(&next_record(Some(&first), "x", 1_030)), "dns", 1_100);
    assert_eq!((third.failures, third.retry_at), (3, 1_220));
    assert_eq!(third.reason, "dns");
}

#[test]
fn codec_runtime_task_background_step_decisions() {
    let failed = next_record(None, "no answer", 1_000);
    // The user's switch wins, then an installed runtime.
    assert_eq!(
        background_step(true, true, false, None, 0),
        Step::Wait(POLL)
    );
    assert_eq!(
        background_step(true, false, true, Some(&failed), 0),
        Step::Installed
    );
    // The start of the daemon attempts at once, even with a record.
    assert_eq!(
        background_step(true, false, false, Some(&failed), 1_001),
        Step::Attempt
    );
    // Later rounds wait for the record's next attempt, at most POLL.
    assert_eq!(
        background_step(false, false, false, Some(&failed), 1_020),
        Step::Wait(Duration::from_secs(10))
    );
    let later = InstallRecord {
        retry_at: 5_000,
        ..failed.clone()
    };
    assert_eq!(
        background_step(false, false, false, Some(&later), 1_020),
        Step::Wait(POLL)
    );
    assert_eq!(
        background_step(false, false, false, Some(&failed), 1_030),
        Step::Attempt
    );
    assert_eq!(
        background_step(false, false, false, None, 1_030),
        Step::Attempt
    );
}

#[test]
fn codec_runtime_task_session_gate_waits_only_right_after_a_failure() {
    let failed = next_record(None, "no answer", 1_000);
    assert_eq!(session_gate(None, 1_000), Ok(()));
    assert_eq!(
        session_gate(Some(&failed), 1_010),
        Err("no answer".to_string())
    );
    assert_eq!(session_gate(Some(&failed), 1_029), Err("no answer".into()));
    assert_eq!(session_gate(Some(&failed), 1_030), Ok(()));
}

#[test]
fn codec_runtime_task_first_contact_gives_up_only_without_bytes() {
    // A healthy attempt: start, first read, bytes.
    let healthy = FirstContact::default();
    assert!(!healthy.cancelled() && !healthy.cancelled());
    healthy.progress(65_536);
    for _ in 0..10 {
        assert!(!healthy.cancelled(), "bytes arrived: never cancelled");
    }
    // A broken transfer after bytes waits through its pauses.
    healthy.progress(65_536);
    assert!(!healthy.gave_up());

    // No connection: the start of the attempt, then the pause asks.
    let offline = FirstContact::default();
    assert!(!offline.cancelled(), "start of the first attempt");
    assert!(!offline.cancelled(), "first question of the pause");
    assert!(
        offline.cancelled(),
        "the pause after a failed first contact"
    );
    assert!(offline.gave_up() && offline.cancelled());

    // A restarted answer that keeps no byte is no progress.
    let restarted = FirstContact::default();
    restarted.progress(0);
    assert!(!restarted.cancelled() && !restarted.cancelled());
    assert!(restarted.cancelled());
}

#[test]
fn codec_runtime_task_record_and_lock() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().join("runtime");
    assert_eq!(read_record(&dir), None);
    assert!(!held_elsewhere(&dir), "no lock file: nothing installs");
    assert!(!dir.exists(), "looking creates nothing");

    let record = next_record(None, "no answer", 1_000);
    write_record(&dir, &record).unwrap();
    assert_eq!(read_record(&dir), Some(record));
    clear_record(&dir).unwrap();
    clear_record(&dir).unwrap();
    assert_eq!(read_record(&dir), None);

    let lock = InstallLock::acquire(&dir).unwrap();
    assert!(held_elsewhere(&dir), "an installation runs");
    drop(lock);
    assert!(!held_elsewhere(&dir));
}

#[test]
fn codec_runtime_task_state_texts() {
    let now = 1_000;
    let text = |report: OpenH264Report| openh264_state_text(&report, now);
    assert_eq!(
        text(OpenH264Report::new(OpenH264State::Installing, None)),
        "wird installiert"
    );
    assert_eq!(
        text(OpenH264Report::new(OpenH264State::Enabled, None)),
        "installiert"
    );
    assert_eq!(
        text(OpenH264Report::new(OpenH264State::Disabled, None)),
        "abgeschaltet"
    );
    assert!(text(OpenH264Report::new(OpenH264State::Absent, None))
        .starts_with("noch nicht installiert – wird beim Start"));
    assert_eq!(
        text(OpenH264Report::new(
            OpenH264State::Absent,
            Some("not offered".into())
        )),
        "nicht angeboten (not offered)"
    );
    let failed = OpenH264Report::new(OpenH264State::Failed, Some("no answer".into()));
    let due = text(failed.clone().with_retry_at(Some(now + 30)));
    assert!(
        due.starts_with("Fehler: no answer – neuer Versuch um "),
        "{due}"
    );
    assert!(due.contains(':'), "{due}");
    assert!(text(failed.clone().with_retry_at(Some(now)))
        .ends_with("neuer Versuch bei der nächsten Sitzung oder im Hintergrunddienst"));
    assert!(text(failed).ends_with("beim nächsten Start des Hintergrunddienstes"));

    let ffmpeg = |state, retry_at| RuntimeReport {
        state,
        version: "n8.1".into(),
        reason: Some("HTTP 503".into()),
        retry_at,
    };
    assert_eq!(
        runtime_state_text(&ffmpeg(RuntimeState::Installed, None), now),
        "installiert"
    );
    let failed = runtime_state_text(&ffmpeg(RuntimeState::Failed, Some(now + 60)), now);
    assert!(failed.starts_with("Fehler: HTTP 503 – neuer Versuch um "));
    assert!(failed.ends_with("bis dahin dekodiert Media Foundation"));
    let json = serde_json::to_value(ffmpeg(RuntimeState::Installing, None)).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"state": "installing", "version": "n8.1", "reason": "HTTP 503"})
    );
}
