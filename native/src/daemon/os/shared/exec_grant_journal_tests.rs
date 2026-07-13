use std::cell::RefCell;

use super::{decode_entry, drive_steps, should_fail_closed, JournalPhase, StepProgress};

#[test]
fn enable_is_persisted_before_runtime_apply_and_ack_clear() {
    let order = RefCell::new(Vec::new());
    let progress = drive_steps(
        JournalPhase::PendingApply,
        || {
            order.borrow_mut().push("persist");
            Ok(())
        },
        || {
            order.borrow_mut().push("apply");
            Ok(())
        },
        || {
            order.borrow_mut().push("clear");
            Ok(())
        },
    );

    assert_eq!(*order.borrow(), ["persist", "apply", "clear"]);
    assert_eq!(
        progress,
        StepProgress {
            persisted: true,
            applied: true,
            cleared: true,
            error: None,
        }
    );
}

#[test]
fn enable_apply_failure_leaves_a_retryable_pending_apply() {
    let order = RefCell::new(Vec::new());
    let progress = drive_steps(
        JournalPhase::PendingApply,
        || {
            order.borrow_mut().push("persist");
            Ok(())
        },
        || {
            order.borrow_mut().push("apply");
            Err("injected apply crash window".into())
        },
        || {
            order.borrow_mut().push("clear");
            Ok(())
        },
    );

    assert_eq!(*order.borrow(), ["persist", "apply"]);
    assert!(progress.persisted);
    assert!(!progress.applied);
    assert!(!progress.cleared);
    assert_eq!(
        progress.error.as_deref(),
        Some("injected apply crash window")
    );
}

#[test]
fn deny_is_applied_before_persistence_and_ack_clear() {
    let order = RefCell::new(Vec::new());
    let progress = drive_steps(
        JournalPhase::PendingDeny,
        || {
            order.borrow_mut().push("persist");
            Ok(())
        },
        || {
            order.borrow_mut().push("apply");
            Ok(())
        },
        || {
            order.borrow_mut().push("clear");
            Ok(())
        },
    );

    assert_eq!(*order.borrow(), ["apply", "persist", "clear"]);
    assert!(progress.persisted && progress.applied && progress.cleared);
}

#[test]
fn deny_persist_failure_never_clears_the_deny_tombstone() {
    let order = RefCell::new(Vec::new());
    let progress = drive_steps(
        JournalPhase::PendingDeny,
        || {
            order.borrow_mut().push("persist");
            Err("injected CAS crash window".into())
        },
        || {
            order.borrow_mut().push("apply");
            Ok(())
        },
        || {
            order.borrow_mut().push("clear");
            Ok(())
        },
    );

    assert_eq!(*order.borrow(), ["apply", "persist"]);
    assert!(progress.applied);
    assert!(!progress.persisted);
    assert!(!progress.cleared);
}

#[test]
fn enable_unlink_sync_failure_restores_pending_and_requires_runtime_stop() {
    let order = RefCell::new(Vec::new());
    let journal_present = RefCell::new(true);
    let progress = drive_steps(
        JournalPhase::PendingApply,
        || {
            order.borrow_mut().push("persist");
            Ok(())
        },
        || {
            order.borrow_mut().push("apply");
            Ok(())
        },
        || {
            super::storage::unlink_and_sync_with_recovery(
                &"pending-enable",
                || {
                    order.borrow_mut().push("unlink");
                    *journal_present.borrow_mut() = false;
                    Ok(())
                },
                || {
                    order.borrow_mut().push("sync");
                    Err("injected directory sync failure".into())
                },
                |_| {
                    order.borrow_mut().push("restore");
                    *journal_present.borrow_mut() = true;
                    Ok(())
                },
                || {
                    order.borrow_mut().push("verify");
                    Ok(*journal_present.borrow())
                },
            )
        },
    );

    assert_eq!(
        *order.borrow(),
        ["persist", "apply", "unlink", "sync", "restore", "verify"]
    );
    assert!(progress.persisted && progress.applied);
    assert!(!progress.cleared);
    assert!(*journal_present.borrow());
    assert!(should_fail_closed(JournalPhase::PendingApply, &progress));
    assert_eq!(
        progress.error.as_deref(),
        Some("injected directory sync failure; recovery journal restored and verified")
    );
}

#[test]
fn legacy_contact_target_migrates_to_the_journal_principals_exact_pins() {
    let encoded = serde_json::to_vec(&serde_json::json!({
        "version": 1,
        "operation_id": "01".repeat(16),
        "phase": "pending_apply",
        "expected_policy_revision": 4,
        "mutation": {
            "target": { "Direct": { "contact_id": "stale-contact" } },
            "principal": {
                "relation_kind": "direct",
                "relation_id": "local-lookup",
                "device_id": "device-a",
                "device_name": "Peer",
                "public_key": "node-a",
                "fingerprint": "fingerprint-a",
                "node_id": "node-a"
            },
            "policy": {
                "enabled": true,
                "policy_revision": 5,
                "changed_at": 12,
                "source_request_id": null,
                "source_decision_revision": null
            },
            "authorization_epoch": 0
        }
    }))
    .unwrap();

    let entry = decode_entry(&encoded).unwrap();
    assert_eq!(entry.version, 2);
    assert_eq!(
        entry.mutation.target,
        crate::share::ExecGrantTarget::Direct {
            device_id: "device-a".into(),
            public_key: "node-a".into(),
            fingerprint: "fingerprint-a".into(),
            node_id: "node-a".into(),
        }
    );
}
