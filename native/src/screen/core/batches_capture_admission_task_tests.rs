//! Source fixtures for the one final remote task suite; never a local entrypoint.
use super::capture_admission::{
    CaptureAuthority, CaptureGeneration, CaptureOwner, CapturePlan, SessionAdmission, SourceKey,
};
use super::capture_grant::{CaptureGrant, CaptureSlice, DelegatedCaptureCredit};
use super::capture_plan::{CapturePhases, HostPhase};
use super::frame_budget::{FrameContract, FrameLimits, FRAME_PROTOCOL, MAX_WORKING_BYTES};
use super::frame_memory::{CapacityCeiling, MemoryFacts, MemoryPool};
use super::host_codec_adapters::{HostCodecPlan, HostContract, HostInputPixels, HostOutputPlan};
use super::pixel_convert::FrameSize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
const MIB: u64 = 1024 * 1024;

fn facts() -> Option<MemoryFacts> {
    Some(MemoryFacts {
        available_physical: 4096 * MIB,
        available_commit: 4096 * MIB,
        available_address_space: 4096 * MIB,
    })
}
fn contract(bytes: u64) -> FrameContract {
    FrameContract {
        version: FRAME_PROTOCOL,
        limits: FrameLimits {
            max_size: FrameSize::new(64, 64),
            max_pixels: 4096,
            working_bytes: bytes,
        },
        hybrid_regions: true,
    }
}
fn plan(session: &str, source: &str, generation: u64, bytes: u64) -> CapturePlan {
    let mut plan = CapturePlan::new(
        SourceKey::new(session, source).unwrap(),
        CaptureGeneration::new(generation),
    );
    plan.add_bytes(CaptureOwner::SourcePixels, bytes, 1, CapturePhases::ALL)
        .unwrap();
    plan
}
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

struct Retirement {
    hold: Option<CaptureSlice>,
    retired: Arc<AtomicBool>,
}
impl Drop for Retirement {
    fn drop(&mut self) {
        drop(self.hold.take());
        self.retired.store(true, Ordering::SeqCst);
    }
}
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
struct NativeResource {
    grant: CaptureGrant,
    observed: Arc<AtomicBool>,
}
impl Drop for NativeResource {
    fn drop(&mut self) {
        self.observed
            .store(self.grant.live_bytes().unwrap() == 4, Ordering::SeqCst);
    }
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
struct VerifiedDispatcher;
impl CaptureAuthority for VerifiedDispatcher {
    fn approve(&self, plan: CapturePlan) -> std::io::Result<CaptureGrant> {
        let credit = DelegatedCaptureCredit::from_verified_parent(
            plan.source().clone(),
            plan.generation(),
            plan.peak_bytes(),
            plan.owner_limits(),
            (),
        )?;
        CaptureGrant::from_delegated(plan, credit)
    }
}
struct SessionRetirement(Arc<AtomicBool>);
impl Drop for SessionRetirement {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
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
