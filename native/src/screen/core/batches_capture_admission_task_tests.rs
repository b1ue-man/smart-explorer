//! Source fixtures for the one final remote task suite; never a local entrypoint.
use super::capture_admission::{CaptureGeneration, CaptureOwner, CapturePlan, SourceKey};
use super::capture_plan::{CapturePhases, HostPhase};
use super::frame_budget::MAX_WORKING_BYTES;
use super::frame_memory::{CapacityCeiling, MemoryPool};
use super::host_codec_adapters::{HostCodecPlan, HostContract, HostInputPixels, HostOutputPlan};
use super::pixel_convert::FrameSize;
use std::sync::Arc;

#[path = "batches_capture_admission_fixture.rs"]
mod fixture;
use fixture::{contract, facts, plan, MIB};
#[path = "batches_capture_admission_lifetime_task_tests.rs"]
mod lifetime;

#[test]
fn batches_task_capture_explicit_cohort_is_not_the_first_small_request() {
    let pool = Arc::new(MemoryPool::default());
    let first = pool
        .admit_session(None, CapacityCeiling::bounded(128 * MIB), 24 * MIB, "first")
        .unwrap();
    let second = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(128 * MIB),
            104 * MIB,
            "second",
        )
        .unwrap();
    assert_eq!(first.working_bytes(), 24 * MIB);
    assert_eq!(second.working_bytes(), 104 * MIB);
    assert!(pool
        .reserve_with_ceiling(None, CapacityCeiling::bounded(128 * MIB), MIB)
        .is_err());
    drop(first);
    drop(second);
    assert_eq!(
        pool.reserve_with_ceiling(None, CapacityCeiling::bounded(32 * MIB), 64 * MIB)
            .unwrap()
            .bytes(),
        32 * MIB
    );
}
#[test]
fn batches_task_capture_classes_and_total_are_checked_together() {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(facts(), CapacityCeiling::desktop(facts()), 64 * MIB, "s")
        .unwrap();
    admission.bind_contract(Some(contract(64 * MIB))).unwrap();
    let mut disjoint =
        CapturePlan::new(SourceKey::new("s", "a").unwrap(), CaptureGeneration::new(1));
    disjoint
        .add_bytes(
            CaptureOwner::SourcePixels,
            6,
            1,
            CapturePhases::one(HostPhase::Capture),
        )
        .unwrap();
    disjoint
        .add_bytes(
            CaptureOwner::CompositionPixels,
            6,
            1,
            CapturePhases::one(HostPhase::LiveEncode),
        )
        .unwrap();
    assert_eq!(disjoint.peak_bytes(), 6);
    let grant = admission.authority().approve(disjoint).unwrap();
    assert!(grant.claim(CaptureOwner::SourcePixels, 7).is_err());
    let first = grant.claim(CaptureOwner::SourcePixels, 4).unwrap();
    assert!(grant.claim(CaptureOwner::CompositionPixels, 3).is_err());
    assert_eq!(grant.live_bytes().unwrap(), 4);
    drop(first);
    assert_eq!(
        grant
            .claim(CaptureOwner::CompositionPixels, 6)
            .unwrap()
            .bytes(),
        6
    );
}
#[test]
fn batches_task_capture_split_and_free_credits_have_no_artificial_fragmentation() {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(facts(), CapacityCeiling::desktop(facts()), 64 * MIB, "s")
        .unwrap();
    admission.bind_contract(Some(contract(64 * MIB))).unwrap();
    let grant = admission
        .authority()
        .approve(plan("s", "a", 1, 10))
        .unwrap();
    let left = grant.claim(CaptureOwner::SourcePixels, 2).unwrap();
    let middle = grant.claim(CaptureOwner::SourcePixels, 6).unwrap();
    let right = grant.claim(CaptureOwner::SourcePixels, 2).unwrap();
    drop(left);
    drop(right);
    let replacement = grant.claim(CaptureOwner::SourcePixels, 4).unwrap();
    let (a, b) = replacement.split(1).unwrap();
    assert_eq!(
        (a.bytes(), b.bytes(), grant.live_bytes().unwrap()),
        (1, 3, 10)
    );
    drop(middle);
    drop(a);
    assert_eq!(grant.live_bytes().unwrap(), 3);
    drop(b);
    assert_eq!(grant.live_bytes().unwrap(), 0);
}
#[test]
fn batches_task_capture_legacy_4k_reuses_w_instead_of_booking_w_plus_c() {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(
            None,
            CapacityCeiling::desktop(None),
            MAX_WORKING_BYTES,
            "legacy",
        )
        .unwrap();
    admission.bind_contract(None).unwrap();
    let size = FrameSize::new(3840, 2160);
    let mut capture = CapturePlan::new(
        SourceKey::new("legacy", "wgc").unwrap(),
        CaptureGeneration::new(0),
    );
    capture
        .add_bgra(CaptureOwner::NativeSurface, size, 2, CapturePhases::ALL)
        .unwrap();
    capture
        .add_bgra(CaptureOwner::Readback, size, 1, CapturePhases::ALL)
        .unwrap();
    capture
        .add_bgra(CaptureOwner::SourcePixels, size, 2, CapturePhases::ALL)
        .unwrap();
    HostOutputPlan::new(HostContract::Legacy, size, HostCodecPlan::Tiles)
        .unwrap()
        .append_to(&mut capture, HostInputPixels::SeparateImage)
        .unwrap();
    assert!(capture.peak_bytes() > 331_882_080);
    assert!(capture.peak_bytes() < MAX_WORKING_BYTES);
    let grant = admission.authority().approve(capture).unwrap();
    assert_eq!(admission.additional_bytes().unwrap(), 0);
    assert_eq!(admission.working_bytes(), MAX_WORKING_BYTES);
    assert!(grant
        .claim(CaptureOwner::NativeSurface, size.pixels() * 8)
        .is_ok());
    assert!(admission
        .bind_contract(Some(contract(MAX_WORKING_BYTES)))
        .is_err());
}
#[test]
fn batches_task_capture_extra_is_exact_and_survives_the_last_result() {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(facts(), CapacityCeiling::bounded(128 * MIB), 64 * MIB, "s")
        .unwrap();
    admission.bind_contract(Some(contract(64 * MIB))).unwrap();
    let first = admission
        .authority()
        .approve(plan("s", "a", 1, 96 * MIB))
        .unwrap();
    assert_eq!(admission.additional_bytes().unwrap(), 32 * MIB);
    assert!(admission
        .authority()
        .approve(plan("s", "b", 1, 40 * MIB))
        .is_err());
    assert_eq!(admission.additional_bytes().unwrap(), 32 * MIB);
    let held = first.claim(CaptureOwner::SourcePixels, MIB).unwrap();
    drop(first);
    assert_eq!(admission.additional_bytes().unwrap(), 32 * MIB);
    drop(held);
    assert_eq!(admission.additional_bytes().unwrap(), 0);
}
#[test]
fn batches_task_capture_source_generation_and_live_identity_retirement() {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(facts(), CapacityCeiling::desktop(facts()), 64 * MIB, "s")
        .unwrap();
    assert!(admission.authority().approve(plan("s", "a", 1, 1)).is_err());
    admission.bind_contract(Some(contract(64 * MIB))).unwrap();
    assert!(admission
        .authority()
        .approve(plan("foreign", "a", 1, 1))
        .is_err());
    let newest = admission.authority().approve(plan("s", "a", 2, 1)).unwrap();
    assert!(newest
        .check_identity(
            &SourceKey::new("s", "different").unwrap(),
            CaptureGeneration::new(2)
        )
        .is_err());
    assert!(admission.authority().approve(plan("s", "a", 1, 1)).is_err());
    drop(newest);
    for index in 0..300 {
        drop(
            admission
                .authority()
                .approve(plan("s", &format!("source-{index}"), 1, 1))
                .unwrap(),
        );
    }
}
#[test]
fn batches_task_capture_family_member_binding_keeps_one_root_total() {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(facts(), CapacityCeiling::desktop(facts()), 64 * MIB, "s")
        .unwrap();
    admission.bind_contract(Some(contract(64 * MIB))).unwrap();
    let mut family = plan("s", "family", 3, 2);
    family.add_member(plan("s", "window-a", 4, 4)).unwrap();
    family.add_member(plan("s", "window-b", 5, 4)).unwrap();
    let root = admission.authority().approve(family).unwrap();
    let key = SourceKey::new("s", "window-a").unwrap();
    let member = root.for_member(&key, CaptureGeneration::new(4)).unwrap();
    let same_member = root.for_member(&key, CaptureGeneration::new(4)).unwrap();
    let first = member.claim(CaptureOwner::SourcePixels, 3).unwrap();
    assert!(same_member.claim(CaptureOwner::SourcePixels, 2).is_err());
    assert!(root.for_member(&key, CaptureGeneration::new(3)).is_err());
    assert_eq!(root.live_bytes().unwrap(), 3);
    drop(first);
    assert!(same_member.claim(CaptureOwner::SourcePixels, 4).is_ok());
}

#[test]
fn batches_task_capture_checked_plan_arithmetic_is_transactional() {
    let mut candidate = plan("s", "a", 1, 1);
    assert!(candidate
        .add_bytes(CaptureOwner::NativeSurface, u64::MAX, 2, CapturePhases::ALL)
        .is_err());
    assert_eq!(candidate.peak_bytes(), 1);
    assert_eq!(candidate.requirements().len(), 1);
}
#[test]
fn batches_task_capture_parallel_alias_grants_share_one_atomic_total() {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(facts(), CapacityCeiling::desktop(facts()), 64 * MIB, "s")
        .unwrap();
    admission.bind_contract(Some(contract(64 * MIB))).unwrap();
    let grant = admission.authority().approve(plan("s", "a", 1, 3)).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let grant = grant.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                grant.claim(CaptureOwner::SourcePixels, 2).ok()
            })
        })
        .collect();
    let held: Vec<_> = threads
        .into_iter()
        .filter_map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(held.len(), 1);
    assert_eq!(grant.live_bytes().unwrap(), 2);
    drop(held);
    assert_eq!(grant.live_bytes().unwrap(), 0);
}
#[test]
fn batches_task_capture_serial_family_sums_persistence_and_maximum_transient() {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(facts(), CapacityCeiling::desktop(facts()), 64 * MIB, "s")
        .unwrap();
    admission.bind_contract(Some(contract(64 * MIB))).unwrap();
    let mut family = CapturePlan::new(
        SourceKey::new("s", "family").unwrap(),
        CaptureGeneration::new(1),
    );
    family
        .add_bytes(CaptureOwner::CompositionPixels, 4, 1, CapturePhases::ALL)
        .unwrap();
    for (name, persistent, transient, owner) in [
        ("a", 2, 8, CaptureOwner::Readback),
        ("b", 3, 6, CaptureOwner::SourcePixels),
    ] {
        let key = SourceKey::new("s", name).unwrap();
        let mut retained = CapturePlan::new(key.clone(), CaptureGeneration::new(1));
        retained
            .add_bytes(
                CaptureOwner::NativeSurface,
                persistent,
                1,
                CapturePhases::ALL,
            )
            .unwrap();
        let mut temporary = CapturePlan::new(key, CaptureGeneration::new(1));
        temporary
            .add_bytes(owner, transient, 1, CapturePhases::one(HostPhase::Capture))
            .unwrap();
        family.add_serial_member(retained, temporary).unwrap();
    }
    assert_eq!(family.phase_bytes(HostPhase::Capture), 4 + 2 + 3 + 8);
    assert_eq!(family.concurrency_cases().len(), 2);
    let grant = admission.authority().approve(family).unwrap();
    let a = grant
        .for_member(
            &SourceKey::new("s", "a").unwrap(),
            CaptureGeneration::new(1),
        )
        .unwrap();
    let b = grant
        .for_member(
            &SourceKey::new("s", "b").unwrap(),
            CaptureGeneration::new(1),
        )
        .unwrap();
    let _canvas = grant.claim(CaptureOwner::CompositionPixels, 4).unwrap();
    let _native_a = a.claim(CaptureOwner::NativeSurface, 2).unwrap();
    let _native_b = b.claim(CaptureOwner::NativeSurface, 3).unwrap();
    let transient_a = a.claim(CaptureOwner::Readback, 8).unwrap();
    assert!(b.claim(CaptureOwner::SourcePixels, 6).is_err());
    drop(transient_a);
    assert_eq!(b.claim(CaptureOwner::SourcePixels, 6).unwrap().bytes(), 6);
}
