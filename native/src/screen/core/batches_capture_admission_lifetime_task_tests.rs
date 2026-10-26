//! Delegated and native resource lifetime cases from the admission contract.
use super::fixture::{NativeResource, Retirement, SessionRetirement, VerifiedDispatcher};
use super::{contract, facts, plan, MIB};
use crate::screen::core::{
    capture_admission::{CaptureOwner, SessionAdmission},
    capture_grant::{CaptureGrant, DelegatedCaptureCredit},
    frame_memory::{CapacityCeiling, MemoryPool},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[test]
fn batches_task_capture_delegation_consumes_one_parent_backing_until_last_slice() {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(facts(), CapacityCeiling::bounded(256 * MIB), 64 * MIB, "s")
        .unwrap();
    admission.bind_contract(Some(contract(64 * MIB))).unwrap();
    let first = admission
        .authority()
        .approve(plan("s", "a", 1, 96 * MIB))
        .unwrap();
    let second_admission = pool
        .admit_session(
            facts(),
            CapacityCeiling::bounded(256 * MIB),
            64 * MIB,
            "second",
        )
        .unwrap();
    second_admission
        .bind_contract(Some(contract(64 * MIB)))
        .unwrap();
    let second = second_admission
        .authority()
        .approve(plan("second", "b", 1, 96 * MIB))
        .unwrap();
    let viewer = pool
        .reserve_with_ceiling(facts(), CapacityCeiling::bounded(256 * MIB), 64 * MIB)
        .unwrap();
    assert_eq!(viewer.bytes(), 64 * MIB);
    assert!(pool
        .reserve_with_ceiling(facts(), CapacityCeiling::bounded(256 * MIB), MIB)
        .is_err());
    let retired = Arc::new(AtomicBool::new(false));
    let worker_plan = plan("s", "a", 1, 96 * MIB);
    let credit = DelegatedCaptureCredit::from_verified_parent(
        first.source().clone(),
        first.generation(),
        worker_plan.peak_bytes(),
        worker_plan.owner_limits(),
        Retirement {
            hold: Some(first.claim(CaptureOwner::SourcePixels, 96 * MIB).unwrap()),
            retired: retired.clone(),
        },
    )
    .unwrap();
    let worker = CaptureGrant::from_delegated(worker_plan, credit).unwrap();
    let worker_clone = worker.clone();
    let result = worker.claim(CaptureOwner::SourcePixels, MIB).unwrap();
    drop(worker);
    drop(worker_clone);
    drop(first);
    assert!(!retired.load(Ordering::SeqCst));
    assert_eq!(admission.additional_bytes().unwrap(), 32 * MIB);
    drop(result);
    assert!(retired.load(Ordering::SeqCst));
    assert_eq!(admission.additional_bytes().unwrap(), 0);
    assert_eq!(second_admission.additional_bytes().unwrap(), 32 * MIB);
    drop(second);
    assert_eq!(second_admission.additional_bytes().unwrap(), 0);
}
#[test]
fn batches_task_capture_delegation_rejects_mismatched_limits() {
    let candidate = plan("s", "a", 1, 10);
    let credit = DelegatedCaptureCredit::from_verified_parent(
        candidate.source().clone(),
        candidate.generation(),
        11,
        candidate.owner_limits(),
        (),
    )
    .unwrap();
    assert!(CaptureGrant::from_delegated(candidate, credit).is_err());
}
#[test]
fn batches_task_capture_native_resource_dies_before_its_last_slice() {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(facts(), CapacityCeiling::desktop(facts()), 64 * MIB, "s")
        .unwrap();
    admission.bind_contract(Some(contract(64 * MIB))).unwrap();
    let grant = admission.authority().approve(plan("s", "a", 1, 4)).unwrap();
    let observed = Arc::new(AtomicBool::new(false));
    let resource = grant
        .claim(CaptureOwner::SourcePixels, 4)
        .unwrap()
        .allocate(|| {
            assert_eq!(grant.live_bytes().unwrap(), 4);
            Ok(NativeResource {
                grant: grant.clone(),
                observed: observed.clone(),
            })
        })
        .unwrap();
    drop(resource);
    assert!(observed.load(Ordering::SeqCst));
    assert_eq!(grant.live_bytes().unwrap(), 0);
}
#[test]
fn batches_task_capture_delegated_session_prepares_and_resizes_without_a_child_pool() {
    let retired = Arc::new(AtomicBool::new(false));
    let session = SessionAdmission::from_verified_parent(
        "worker",
        64 * MIB,
        Some(contract(64 * MIB)),
        Arc::new(VerifiedDispatcher),
        SessionRetirement(retired.clone()),
    )
    .unwrap();
    assert_eq!(session.session(), "worker");
    assert_eq!(session.working_bytes(), 64 * MIB);
    let authority = session.authority();
    assert!(authority
        .approve(plan("wrong-session", "a", 1, 10))
        .is_err());
    let first = authority.approve(plan("worker", "a", 1, 10)).unwrap();
    let second = authority.approve(plan("worker", "a", 2, 20)).unwrap();
    let held = first.claim(CaptureOwner::SourcePixels, 10).unwrap();
    drop(session);
    drop(authority);
    drop(first);
    drop(second);
    assert!(!retired.load(Ordering::SeqCst));
    drop(held);
    assert!(retired.load(Ordering::SeqCst));
}
