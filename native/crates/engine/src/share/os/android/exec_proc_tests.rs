//! Host tests of the Android exec host's pure `/proc` logic. Nothing here
//! signals or reaps a real process: the shared test process must never kill
//! or collect children of other tests.
use super::exec_proc::{
    descendants, is_recorded_root, kill_plan, parse_record, parse_stat, record_text, KillPlan,
    ProcStat,
};

fn stat(pid: i32, ppid: i32, pgrp: i32, state: u8) -> ProcStat {
    ProcStat {
        pid,
        state,
        ppid,
        pgrp,
        start_time: 1000 + pid as u64,
    }
}

#[test]
fn android_task_exec_proc_stat_reads_fields_after_the_last_parenthesis() {
    let line =
        "4242 (sh -c (x) y) S 4100 4242 4242 0 -1 4194560 120 0 0 0 1 2 0 0 20 -5 1 0 987654 \
                10000000 200 18446744073709551615 1 1 0 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0\n";
    let parsed = parse_stat(line).expect("stat line");
    assert_eq!(
        parsed,
        ProcStat {
            pid: 4242,
            state: b'S',
            ppid: 4100,
            pgrp: 4242,
            start_time: 987654,
        }
    );
    let zombie =
        parse_stat("7 ()) Z 1 7 7 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 55 0 0").expect("zombie");
    assert_eq!(
        (zombie.state, zombie.ppid, zombie.start_time),
        (b'Z', 1, 55)
    );
}

#[test]
fn android_task_exec_proc_stat_rejects_malformed_lines() {
    for line in [
        "",
        "12 sleep S 1 12 12",
        "12 (sleep S 1 12 12",
        "x (sleep) S 1 12 12 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0 5",
        "12 (sleep) SS 1 12 12 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0 5",
        "12 (sleep) S 1 12 12 0 0 0 0 0 0",
        ") 12 (sleep S 1 12 12 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0 5",
    ] {
        assert_eq!(parse_stat(line), None, "{line:?}");
    }
}

/// The app (100) runs a sync hook (150 → 151) beside the job's intermediate
/// (200). Below it: the shell 201 in its own group with a child 202, a
/// `setsid` child 203 that leads group 203, an orphan 204 already
/// reparented to the intermediate, a zombie 205, and 206 forked by 203.
fn table() -> Vec<ProcStat> {
    vec![
        stat(1, 0, 1, b'S'),
        stat(100, 1, 100, b'S'),
        stat(150, 100, 100, b'S'),
        stat(151, 150, 100, b'S'),
        stat(200, 100, 100, b'S'),
        stat(201, 200, 201, b'S'),
        stat(202, 201, 201, b'S'),
        stat(203, 201, 203, b'S'),
        stat(204, 200, 201, b'S'),
        stat(205, 200, 201, b'Z'),
        stat(206, 203, 203, b'R'),
        stat(300, 1, 300, b'S'),
    ]
}

#[test]
fn android_task_exec_proc_descendants_follow_setsid_children_and_orphans() {
    let below: Vec<i32> = descendants(&table(), 200).into_iter().collect();
    assert_eq!(below, [201, 202, 203, 204, 205, 206]);
    assert!(descendants(&table(), 206).is_empty());
    // A torn snapshot with a cycle ends instead of looping.
    let cycle = vec![stat(10, 11, 10, b'S'), stat(11, 10, 10, b'S')];
    assert_eq!(
        descendants(&cycle, 10).into_iter().collect::<Vec<_>>(),
        [11]
    );
}

#[test]
fn android_task_exec_proc_kill_plan_spares_the_app_hooks_and_the_intermediate() {
    let plan = kill_plan(&table(), 200, 100);
    assert_eq!(
        plan,
        KillPlan {
            pids: vec![201, 202, 203, 204, 206],
            groups: vec![201, 203],
        }
    );
    // The hook's shell and child (150/151), the app (100), init and the
    // intermediate (200) are never signalled.
    for spared in [1, 100, 150, 151, 200, 300] {
        assert!(!plan.pids.contains(&spared) && !plan.groups.contains(&spared));
    }
    // A group whose leader already ended is not signalled as a group: its id
    // could belong to somebody else by now.
    let orphaned_group = vec![stat(200, 100, 100, b'S'), stat(210, 200, 209, b'S')];
    assert_eq!(
        kill_plan(&orphaned_group, 200, 100),
        KillPlan {
            pids: vec![210],
            groups: vec![],
        }
    );
    assert_eq!(kill_plan(&table(), 999, 100), KillPlan::default());
    // Even a snapshot that lists the app below the intermediate spares it.
    let confused = vec![stat(200, 1, 200, b'S'), stat(100, 200, 100, b'S')];
    assert_eq!(kill_plan(&confused, 200, 100), KillPlan::default());
}

#[test]
fn android_task_exec_proc_crash_record_matches_only_the_same_process() {
    let text = record_text(4321, 987654);
    assert_eq!(text, "4321 987654\n");
    assert_eq!(parse_record(&text), Some((4321, 987654)));
    for broken in ["", "4321", "4321 x", "4321 1 2", "1 55", "-3 55", "abc 1"] {
        assert_eq!(parse_record(broken), None, "{broken:?}");
    }

    let running = parse_stat("4321 (se-exec) S 1 4321 4321 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 987654")
        .expect("stat");
    assert!(is_recorded_root(Some(&running), 4321, 987654));
    // The pid was reused by a later process, the process is gone, or only
    // its zombie is left: nothing to end.
    assert!(!is_recorded_root(Some(&running), 4321, 987653));
    assert!(!is_recorded_root(None, 4321, 987654));
    let zombie = ProcStat {
        state: b'Z',
        ..running
    };
    assert!(!is_recorded_root(Some(&zombie), 4321, 987654));
}
