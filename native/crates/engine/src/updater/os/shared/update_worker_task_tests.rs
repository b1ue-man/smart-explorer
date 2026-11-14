//! E24: a worker that ran before `se update` starts once after the commit;
//! without a worker before the update, none starts.
use std::cell::Cell;

use super::restart_if_running;
use crate::daemon::WorkerHandoff;

#[test]
fn update_worker_task_running_worker_starts_once_and_is_replaced() {
    let starts = Cell::new(0);
    let handoff = restart_if_running(true, || {
        starts.set(starts.get() + 1);
        Ok(())
    });
    assert_eq!(handoff, Ok(Some(WorkerHandoff::Replaced)));
    assert_eq!(starts.get(), 1);
}

#[test]
fn update_worker_task_without_running_worker_nothing_starts() {
    let starts = Cell::new(0);
    let handoff = restart_if_running(false, || {
        starts.set(starts.get() + 1);
        Ok(())
    });
    assert_eq!(handoff, Ok(None));
    assert_eq!(starts.get(), 0);
}

#[test]
fn update_worker_task_failed_start_is_an_error_not_a_replacement() {
    let handoff = restart_if_running(true, || Err("no worker answered".into()));
    assert_eq!(handoff, Err("no worker answered".to_string()));
}
