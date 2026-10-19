//! A vault job started on the desktop runs only through the vault sync
//! (C5.8, VLT-M7) and leaves no state for the general conflict resolution.
use std::cell::Cell;
use std::sync::atomic::AtomicBool;

use super::run_vault;
use crate::syncjobs::SyncJob;
use crate::vault::{VaultRunOutcome, VaultRunRequest};

#[test]
fn vault_task_desktop_runs_vault_job_through_vault_sync() {
    let job = SyncJob::new_vault(
        "Tresor".into(),
        "0123456789abcdef0123456789abcdef",
        "fedcba9876543210fedcba9876543210",
    );
    let called = Cell::new(false);
    let runner = |request: VaultRunRequest<'_>| {
        called.set(true);
        assert_eq!(request.job.id, job.id);
        assert!(!request.dry_run);
        let mut outcome = VaultRunOutcome::default();
        outcome.outcome.stats.a_to_b = 1;
        outcome.outcome.state = Some(crate::bisync::StateKey::legacy("pair", "lock"));
        outcome
    };
    let cancel = AtomicBool::new(false);
    let settings = crate::bisync::RunSettings::for_job(&job.id);
    let out = run_vault(&job, &cancel, settings, &runner).unwrap();
    assert!(called.get());
    assert_eq!(out.stats.a_to_b, 1);
    assert!(out.state.is_none());
    let refuse = |_: VaultRunRequest<'_>| VaultRunOutcome {
        unavailable: Some("Tresor-Job: anderer Tresor".into()),
        ..Default::default()
    };
    let settings = crate::bisync::RunSettings::for_job(&job.id);
    let error = run_vault(&job, &cancel, settings, &refuse).unwrap_err();
    assert!(error.contains("anderer Tresor"));
}
