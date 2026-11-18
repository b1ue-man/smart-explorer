//! The stop of the installed image for an update (field failure 2026-11-18).
use super::*;
use std::cell::{Cell, RefCell};

const FAST: Limits = Limits {
    round: Duration::from_millis(40),
    poll: Duration::from_millis(1),
    restops: 2,
};

fn target(pid: u32) -> Found {
    Found::Target {
        pid,
        image: r"\\?\C:\Programs\Smart Explorer\se.exe".into(),
    }
}

#[test]
fn update_mcp_task_a_restarted_service_is_stopped_again() {
    // The worker 10 stops; a fresh start (20) consumes the request, which is
    // renewed once; then nothing runs any more.
    let scans = RefCell::new(vec![
        vec![],
        vec![target(20)],
        vec![target(20)],
        vec![target(10)],
    ]);
    let pending = Cell::new(true);
    let restops = Cell::new(0);
    let result = stop_until_gone(
        FAST,
        false,
        &mut || {
            let next = scans.borrow_mut().pop().unwrap_or_default();
            if next.iter().any(|f| *f == target(20)) && restops.get() == 0 {
                pending.set(false);
            }
            Ok(next)
        },
        &|| pending.get(),
        &mut || {
            restops.set(restops.get() + 1);
            pending.set(true);
            Ok(())
        },
    );
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(restops.get(), 1);
}

#[test]
fn update_mcp_task_a_service_restarted_without_end_is_reported_with_pid_and_path() {
    let next_pid = Cell::new(100);
    let restops = Cell::new(0);
    let error = stop_until_gone(
        FAST,
        false,
        &mut || {
            next_pid.set(next_pid.get() + 1);
            Ok(vec![target(next_pid.get())])
        },
        &|| true,
        &mut || {
            restops.set(restops.get() + 1);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(restops.get(), FAST.restops);
    assert!(!error.needs_elevation, "{}", error.msg);
    assert!(
        error.msg.contains("laeuft noch und blockiert das Update"),
        "{}",
        error.msg
    );
    assert!(
        error.msg.contains(r"Smart Explorer\se.exe"),
        "{}",
        error.msg
    );
    assert!(error.msg.contains("PID 104"), "{}", error.msg);
    assert!(
        error.msg.contains("nach 2 erneuten Stop-Anforderungen"),
        "{}",
        error.msg
    );
}

#[test]
fn update_mcp_task_elevation_is_named_only_for_unreadable_processes() {
    let cases = [
        ("user process", vec![target(7)], false, false),
        (
            "user process beside an unreadable one",
            vec![target(7), Found::Unreadable { pid: 8, code: 5 }],
            false,
            false,
        ),
        (
            "only unreadable",
            vec![Found::Unreadable { pid: 8, code: 5 }],
            false,
            true,
        ),
        (
            "unreadable while elevated",
            vec![Found::Unreadable { pid: 8, code: 5 }],
            true,
            false,
        ),
        (
            "other error code",
            vec![Found::Unreadable { pid: 8, code: 31 }],
            false,
            false,
        ),
    ];
    for (case, found, elevated, expected) in cases {
        let error = report(&found, 0, elevated);
        assert_eq!(error.needs_elevation, expected, "{case}: {}", error.msg);
        for process in &found {
            assert!(
                error.msg.contains(&format!("PID {} (", process.pid())),
                "{case}: {}",
                error.msg
            );
        }
    }
    let error = report(&[Found::Unreadable { pid: 8, code: 5 }], 0, false);
    assert!(
        error
            .msg
            .contains("Betriebssystemfehler 5 Zugriff verweigert"),
        "{}",
        error.msg
    );
}

#[test]
fn update_mcp_task_stop_steps() {
    let cases = [
        ((false, true, false, true), Step::Wait),
        ((false, true, true, true), Step::GiveUp),
        ((true, true, false, true), Step::Restop),
        ((false, false, false, true), Step::Restop),
        ((true, true, false, false), Step::GiveUp),
        ((false, false, false, false), Step::GiveUp),
    ];
    for ((fresh, pending, over, may), expected) in cases {
        assert_eq!(
            next_step(fresh, pending, over, may),
            expected,
            "fresh={fresh} pending={pending} over={over} may={may}"
        );
    }
}

#[test]
fn update_mcp_task_a_transient_unreadable_process_does_not_abort() {
    // A process in its exit is briefly not readable; it is waited for.
    let scans = RefCell::new(vec![vec![], vec![Found::Unreadable { pid: 9, code: 5 }]]);
    let result = stop_until_gone(
        FAST,
        false,
        &mut || Ok(scans.borrow_mut().pop().unwrap_or_default()),
        &|| true,
        &mut || Ok(()),
    );
    assert!(result.is_ok(), "{result:?}");
}
