//! A vault job on Android runs only through the vault sync (C5.8, VLT-M7):
//! never through the general engine options, and its conflicts carry no
//! run state, so the general resolution refuses them.
use std::cell::Cell;
use std::sync::atomic::AtomicBool;

use super::{checked_settings, run_vault};
use crate::syncjobs::{SyncJob, VAULT_JOB_NEEDS_VAULT_SYNC};
use crate::vault::{VaultRunOutcome, VaultRunRequest};

fn vault_job() -> SyncJob {
    SyncJob::new_vault(
        "Tresor".into(),
        "0123456789abcdef0123456789abcdef",
        "fedcba9876543210fedcba9876543210",
    )
}

#[test]
fn vault_task_android_runs_vault_job_through_vault_sync() {
    let job = vault_job();
    // The general check no longer refuses the vault job before its run.
    assert!(checked_settings(&job, false).is_ok());
    assert!(matches!(job.checked_opts(false), Err(error) if error == VAULT_JOB_NEEDS_VAULT_SYNC));
    let called = Cell::new(false);
    let runner = |request: VaultRunRequest<'_>| {
        called.set(true);
        assert_eq!(request.job.id, job.id);
        assert!(request.dry_run);
        let mut outcome = VaultRunOutcome::default();
        outcome.outcome.stats.b_to_a = 3;
        outcome.outcome.state = Some(crate::bisync::StateKey::legacy("pair", "lock"));
        outcome
    };
    let cancel = AtomicBool::new(false);
    let settings = crate::bisync::RunSettings::for_job(&job.id);
    let out = run_vault(&job, true, &cancel, settings, &runner).unwrap();
    assert!(called.get());
    assert_eq!(out.stats.b_to_a, 3);
    // Without its state the general resolution and merge refuse the
    // conflicts of a vault job (`sync_conflicts::resolve`).
    assert!(out.state.is_none());
}

#[test]
fn vault_task_android_unavailable_vault_run_changes_nothing() {
    let job = vault_job();
    let refuse = |_: VaultRunRequest<'_>| VaultRunOutcome {
        unavailable: Some("Tresor-Job: anderer Tresor".into()),
        ..Default::default()
    };
    let cancel = AtomicBool::new(false);
    let settings = crate::bisync::RunSettings::for_job(&job.id);
    let error = run_vault(&job, false, &cancel, settings, &refuse).unwrap_err();
    assert_eq!(error.kind, "network");
    assert!(error.message.contains("anderer Tresor"));
}
