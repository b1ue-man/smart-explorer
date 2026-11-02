//! Local-image/retirement fixtures for the single final remote task suite.
use super::capture_admission::{CaptureGeneration, CaptureOwner, CapturePlan, SourceKey};
use super::capture_grant::{CaptureGrant, DelegatedCaptureCredit};
use super::capture_image::{AdmittedBytes, CaptureImageBuilder};
use super::capture_plan::{CapturePhases, OWNER_COUNT};
use super::frame_budget::{FrameContract, FrameLimits, FRAME_PROTOCOL};
use super::frame_memory::{CapacityCeiling, MemoryPool};
use super::image_admission::{ImageAdmission, ImageOwnerLimits, ImageRole, LocalImagePolicy};
use super::pixel_convert::FrameSize;
use std::io::ErrorKind;
use std::sync::{Arc, Barrier, Mutex, Weak};

const MIB: u64 = 1024 * 1024;
fn admission(
    pool: &Arc<MemoryPool>,
    capacity: u64,
    total: u64,
    role: ImageRole,
    bytes_each: u64,
    count: u64,
) -> Arc<ImageAdmission> {
    let limits = ImageOwnerLimits::new(total)
        .unwrap()
        .with_role(role, bytes_each, count)
        .unwrap();
    ImageAdmission::in_pool(
        pool.clone(),
        LocalImagePolicy {
            facts: None,
            ceiling: CapacityCeiling::bounded(capacity),
        },
        limits,
    )
}
fn plan(bytes: u64, generation: u64) -> CapturePlan {
    let mut plan = CapturePlan::new(
        SourceKey::new("image-fixture", "screen").unwrap(),
        CaptureGeneration::new(generation),
    );
    plan.add_bytes(CaptureOwner::SourcePixels, bytes, 1, CapturePhases::ALL)
        .unwrap();
    plan
}
#[test]
fn batches_task_image_claims_are_exact_and_atomic_across_roles_and_root() {
    let pool = Arc::new(MemoryPool::default());
    let limits = ImageOwnerLimits::new(100)
        .unwrap()
        .with_role(ImageRole::ViewerInput, 80, 2)
        .unwrap()
        .with_role(ImageRole::ColorSource, 80, 2)
        .unwrap();
    let images = ImageAdmission::in_pool(
        pool.clone(),
        LocalImagePolicy {
            facts: None,
            ceiling: CapacityCeiling::bounded(200),
        },
        limits,
    );
    let first = images.try_claim(ImageRole::ViewerInput, 60).unwrap();
    assert_eq!(
        images
            .try_claim(ImageRole::ColorSource, 41)
            .err()
            .unwrap()
            .kind(),
        ErrorKind::WouldBlock
    );
    assert_eq!(images.live_count(ImageRole::ColorSource).unwrap(), 0);
    assert_eq!(images.live_bytes().unwrap(), 60);
    assert_eq!(pool.committed_bytes(), 60);
    assert_ne!(
        images
            .try_claim(ImageRole::ViewerInput, 81)
            .err()
            .unwrap()
            .kind(),
        ErrorKind::WouldBlock
    );
    assert_ne!(
        images
            .try_claim(ImageRole::AndroidBitmap, 1)
            .err()
            .unwrap()
            .kind(),
        ErrorKind::WouldBlock
    );
    let second = images.try_claim(ImageRole::ColorSource, 40).unwrap();
    assert_eq!(pool.committed_bytes(), 100);
    drop(first);
    drop(second);
    assert_eq!(images.live_bytes().unwrap(), 0);
    assert_eq!(pool.committed_bytes(), 0);
    assert!(ImageOwnerLimits::new(u64::MAX)
        .unwrap()
        .with_role(ImageRole::ViewerInput, u64::MAX, 2)
        .is_err());
}
#[test]
fn batches_task_image_small_first_claim_does_not_freeze_the_platform_envelope() {
    let pool = Arc::new(MemoryPool::default());
    let images = admission(&pool, 1000, 1000, ImageRole::ViewerInput, 900, 2);
    let small = images.try_claim(ImageRole::ViewerInput, 1).unwrap();
    let large = images.try_claim(ImageRole::ViewerInput, 900).unwrap();
    assert_eq!(pool.committed_bytes(), 901);
    assert_eq!(
        images
            .try_claim(ImageRole::ViewerInput, 1)
            .err()
            .unwrap()
            .kind(),
        ErrorKind::WouldBlock
    );
    drop(small);
    drop(large);
    let screen = pool
        .reserve_with_ceiling(None, CapacityCeiling::bounded(1000), 600)
        .unwrap();
    assert_eq!(
        images
            .try_claim(ImageRole::ViewerInput, 401)
            .err()
            .unwrap()
            .kind(),
        ErrorKind::WouldBlock
    );
    assert_eq!(images.live_count(ImageRole::ViewerInput).unwrap(), 0);
    let allowed = images.try_claim(ImageRole::ViewerInput, 400).unwrap();
    assert_eq!(pool.committed_bytes(), 1000);
    drop(screen);
    drop(allowed);
    assert_eq!(pool.committed_bytes(), 0);
}
#[test]
fn batches_task_image_observation_bounds_stale_controllers_and_all_pool_paths() {
    let pool = Arc::new(MemoryPool::default());
    let images = admission(&pool, 1000, 1000, ImageRole::ViewerInput, 900, 4);
    let existing = images.try_claim(ImageRole::ViewerInput, 300).unwrap();
    pool.observe_capacity_ceiling(CapacityCeiling::bounded(200))
        .unwrap();
    assert_eq!(pool.committed_bytes(), 300);
    assert_eq!(
        images
            .try_claim(ImageRole::ViewerInput, 1)
            .err()
            .unwrap()
            .kind(),
        ErrorKind::WouldBlock
    );
    assert!(pool.reserve(None, 1000).is_err());
    assert!(pool
        .reserve_with_ceiling(None, CapacityCeiling::bounded(1000), 1000)
        .is_err());
    pool.observe_capacity_ceiling(CapacityCeiling::bounded(900))
        .unwrap();
    assert_eq!(
        images
            .try_claim(ImageRole::ViewerInput, 1)
            .err()
            .unwrap()
            .kind(),
        ErrorKind::WouldBlock
    );
    drop(existing);
    // A new observation, after real owners end, can start a fresh cohort.
    pool.observe_capacity_ceiling(CapacityCeiling::bounded(900))
        .unwrap();
    let fresh = images.try_claim(ImageRole::ViewerInput, 800).unwrap();
    assert_eq!(pool.committed_bytes(), 800);
    drop(fresh);
    let working = pool
        .reserve_with_ceiling(None, CapacityCeiling::bounded(900), 100)
        .unwrap();
    let mut extra = working.empty_sibling();
    pool.observe_capacity_ceiling(CapacityCeiling::bounded(120))
        .unwrap();
    assert!(extra.resize_exact(21).is_err());
    extra.resize_exact(20).unwrap();
    assert_eq!(pool.committed_bytes(), 120);
}
#[test]
fn batches_task_image_split_merge_preserves_one_slot_and_releases_each_byte_once() {
    let pool = Arc::new(MemoryPool::default());
    let images = admission(&pool, 1000, 1000, ImageRole::ViewerInput, 100, 1);
    let slice = images.try_claim(ImageRole::ViewerInput, 100).unwrap();
    let (first, second) = slice.split(30).unwrap();
    assert_eq!(pool.committed_bytes(), 100);
    assert_eq!(images.live_count(ImageRole::ViewerInput).unwrap(), 1);
    let merged = first.merge(second).unwrap();
    assert_eq!(merged.bytes(), 100);
    let (first, second) = merged.split(30).unwrap();
    drop(first);
    assert_eq!(pool.committed_bytes(), 70);
    assert_eq!(images.live_count(ImageRole::ViewerInput).unwrap(), 1);
    assert!(images.try_claim(ImageRole::ViewerInput, 1).is_err());
    drop(second);
    assert_eq!(pool.committed_bytes(), 0);
    assert_eq!(images.live_count(ImageRole::ViewerInput).unwrap(), 0);
}
#[test]
fn batches_task_image_bytes_and_pixels_are_immutable_shared_owners() {
    let pool = Arc::new(MemoryPool::default());
    let images = admission(&pool, 1000, 1000, ImageRole::ViewerInput, 32, 2);
    let mut bytes =
        AdmittedBytes::from_slice(images.try_claim(ImageRole::ViewerInput, 16).unwrap()).unwrap();
    bytes.as_mut_slice()[0] = 9;
    assert!(bytes.resize(17).is_err());
    let bytes = bytes.freeze();
    let alias = bytes.clone();
    assert!(bytes.shares_bytes(&alias));
    drop(bytes);
    assert_eq!(alias.as_slice()[0], 9);
    assert_eq!(pool.committed_bytes(), 16);
    drop(alias);
    let mut builder = CaptureImageBuilder::new_local(
        FrameSize::new(2, 2),
        FrameLimits::LEGACY,
        &images,
        ImageRole::ViewerInput,
        SourceKey::new("login", "frame").unwrap(),
        CaptureGeneration::new(1),
    )
    .unwrap();
    builder.pixels_mut()[0] = 7;
    assert_eq!(builder.view().size(), FrameSize::new(2, 2));
    let image = builder.freeze();
    let alias = image.clone();
    assert_eq!(image, alias);
    assert!(image.shares_pixels(&alias));
    assert_eq!(image.image_role(), Some(ImageRole::ViewerInput));
    assert!(Arc::ptr_eq(&image.image_admission().unwrap(), &images));
    assert!(image.grant().claim(CaptureOwner::SourcePixels, 16).is_err());
    drop(image);
    assert_eq!(pool.committed_bytes(), 16);
    assert_eq!(alias.image().data()[0], 7);
    drop(alias);
    assert_eq!(pool.committed_bytes(), 0);
}
#[test]
fn batches_task_image_builder_rejects_foreign_admission_and_denies_before_allocate() {
    let pool = Arc::new(MemoryPool::default());
    let own = admission(&pool, 1000, 1000, ImageRole::ViewerInput, 16, 1);
    let foreign = admission(&pool, 1000, 1000, ImageRole::ViewerInput, 16, 1);
    let bytes =
        AdmittedBytes::from_slice(foreign.try_claim(ImageRole::ViewerInput, 16).unwrap()).unwrap();
    assert!(CaptureImageBuilder::from_local_bytes(
        FrameSize::new(2, 2),
        FrameLimits::LEGACY,
        &own,
        SourceKey::new("login", "frame").unwrap(),
        CaptureGeneration::new(1),
        bytes
    )
    .is_err());
    assert_eq!(pool.committed_bytes(), 0);
    let held = own.try_claim(ImageRole::ViewerInput, 16).unwrap();
    let called = std::cell::Cell::new(false);
    let denied = own.try_claim(ImageRole::ViewerInput, 16).and_then(|slice| {
        slice.allocate(|| {
            called.set(true);
            Ok(())
        })
    });
    assert!(denied.is_err());
    assert!(!called.get());
    drop(held);
}
#[test]
fn batches_task_capture_retirement_keeps_only_live_slices_and_forbids_new_claims() {
    let pool = Arc::new(MemoryPool::default());
    let session = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(4 * MIB),
            MIB,
            "image-fixture",
        )
        .unwrap();
    session
        .bind_contract(Some(FrameContract {
            version: FRAME_PROTOCOL,
            limits: FrameLimits {
                max_size: FrameSize::new(16, 16),
                max_pixels: 256,
                working_bytes: MIB,
            },
            hybrid_regions: true,
        }))
        .unwrap();
    let old = session.authority().approve(plan(3 * MIB, 1)).unwrap();
    let live = old.claim(CaptureOwner::SourcePixels, 64).unwrap();
    assert_eq!(pool.committed_bytes(), 3 * MIB);
    old.retire().unwrap();
    old.retire().unwrap();
    assert_eq!(pool.committed_bytes(), MIB);
    assert_ne!(
        old.claim(CaptureOwner::SourcePixels, 1)
            .err()
            .unwrap()
            .kind(),
        ErrorKind::WouldBlock
    );
    let replacement = session.authority().approve(plan(3 * MIB, 2)).unwrap();
    assert_eq!(pool.committed_bytes(), 3 * MIB + 64);
    let (prefix, suffix) = live.split(16).unwrap();
    drop(prefix);
    assert_eq!(pool.committed_bytes(), 3 * MIB + 48);
    drop(suffix);
    assert_eq!(pool.committed_bytes(), 3 * MIB);
    drop(old);
    drop(replacement);
    drop(session);
    assert_eq!(pool.committed_bytes(), 0);
}
#[test]
fn batches_task_capture_delegated_retirement_notifies_only_real_retired_ownership() {
    let plan = plan(100, 1);
    let seen = Arc::new(Mutex::new(Vec::<[u64; OWNER_COUNT]>::new()));
    let output = seen.clone();
    let access = Arc::new(Mutex::new(None::<Weak<CaptureGrant>>));
    let inspect = access.clone();
    let credit = DelegatedCaptureCredit::from_verified_parent_observed(
        plan.source().clone(),
        plan.generation(),
        plan.peak_bytes(),
        plan.owner_limits(),
        (),
        Arc::new(move |owners| {
            if let Some(grant) = inspect.lock().unwrap().as_ref().and_then(Weak::upgrade) {
                assert_eq!(grant.live_bytes().unwrap(), owners.iter().sum::<u64>());
            }
            output.lock().unwrap().push(owners);
        }),
    )
    .unwrap();
    let grant = Arc::new(CaptureGrant::from_delegated(plan, credit).unwrap());
    *access.lock().unwrap() = Some(Arc::downgrade(&grant));
    let early = grant.claim(CaptureOwner::SourcePixels, 10).unwrap();
    drop(early);
    assert!(seen.lock().unwrap().is_empty());
    let slice = grant.claim(CaptureOwner::SourcePixels, 60).unwrap();
    grant.retire().unwrap();
    assert_eq!(
        seen.lock().unwrap().last().unwrap()[CaptureOwner::SourcePixels as usize],
        60
    );
    let (first, second) = slice.split(20).unwrap();
    drop(first);
    assert_eq!(
        seen.lock().unwrap().last().unwrap()[CaptureOwner::SourcePixels as usize],
        40
    );
    drop(second);
    assert_eq!(
        seen.lock().unwrap().last().unwrap()[CaptureOwner::SourcePixels as usize],
        0
    );
}

#[test]
fn batches_task_image_competing_claims_never_partially_acquire_one_slot() {
    let pool = Arc::new(MemoryPool::default());
    let images = admission(&pool, 1000, 100, ImageRole::ViewerInput, 100, 1);
    let start = Arc::new(Barrier::new(3));
    let acquired = Arc::new(Barrier::new(3));
    let release = Arc::new(Barrier::new(3));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let (images, start, acquired, release) = (
                images.clone(),
                start.clone(),
                acquired.clone(),
                release.clone(),
            );
            std::thread::spawn(move || {
                start.wait();
                let claim = images.try_claim(ImageRole::ViewerInput, 100);
                acquired.wait();
                release.wait();
                claim.is_ok()
            })
        })
        .collect();
    start.wait();
    acquired.wait();
    assert_eq!(pool.committed_bytes(), 100);
    assert_eq!(images.live_count(ImageRole::ViewerInput).unwrap(), 1);
    release.wait();
    let admitted = threads
        .into_iter()
        .map(|thread| usize::from(thread.join().unwrap()))
        .sum::<usize>();
    assert_eq!(admitted, 1);
    assert_eq!(pool.committed_bytes(), 0);
}

#[test]
fn batches_task_image_split_cannot_create_extra_objects_in_one_role_slot() {
    let pool = Arc::new(MemoryPool::default());
    let images = admission(&pool, 1000, 100, ImageRole::AndroidBitmap, 100, 1);
    let (first, second) = images
        .try_claim(ImageRole::AndroidBitmap, 100)
        .unwrap()
        .split(40)
        .unwrap();
    let called = std::cell::Cell::new(false);
    assert!(first
        .allocate(|| {
            called.set(true);
            Ok(())
        })
        .is_err());
    assert!(!called.get());
    assert_eq!(pool.committed_bytes(), 60);
    let object = second
        .allocate(|| {
            called.set(true);
            Ok(())
        })
        .unwrap();
    assert!(called.get());
    assert_eq!(images.live_count(ImageRole::AndroidBitmap).unwrap(), 1);
    drop(object);
    assert_eq!(pool.committed_bytes(), 0);
}
