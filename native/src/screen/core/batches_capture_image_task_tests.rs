//! Admission/allocation lifetime fixtures, executed only by the final remote suite.
use std::sync::Arc;
use super::capture_admission::{CaptureGeneration, CaptureOwner, CapturePlan, SourceKey};
use super::capture_image::{AdmittedBytes, AdmittedRegions, CaptureImageBuilder};
use super::capture_plan::CapturePhases;
use super::frame_budget::{FrameContract, FrameLimits, FRAME_PROTOCOL};
use super::frame_memory::{CapacityCeiling, MemoryPool};
use super::pixel_convert::{BgraRotation, FrameSize, Rect};

fn grant(pixel_bytes: u64, metadata_bytes: u64) -> super::capture_grant::CaptureGrant {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool.admit_session(None, CapacityCeiling::bounded(1024 * 1024), 1024 * 1024, "images").unwrap();
    admission.bind_contract(Some(FrameContract {version: FRAME_PROTOCOL, limits: FrameLimits {
        max_size: FrameSize::new(16, 16), max_pixels: 256, working_bytes: 1024 * 1024}, hybrid_regions: true})).unwrap();
    let mut plan = CapturePlan::new(SourceKey::new("images", "synthetic").unwrap(), CaptureGeneration::new(7));
    plan.add_bytes(CaptureOwner::SourcePixels, pixel_bytes, 1, CapturePhases::ALL).unwrap();
    if metadata_bytes > 0 {plan.add_bytes(CaptureOwner::Metadata, metadata_bytes, 1, CapturePhases::ALL).unwrap();}
    admission.authority().approve(plan).unwrap()
}
#[test]
fn batches_task_capture_image_aliases_share_pixels_and_one_slice() {
    let grant = grant(32, 0);
    let mut builder = CaptureImageBuilder::new(FrameSize::new(2, 2), FrameLimits::LEGACY, &grant, CaptureOwner::SourcePixels).unwrap();
    builder.pixels_mut()[0] = 19;
    assert!(builder.row_mut(2).is_none());
    let image = builder.freeze();
    let alias = image.clone();
    assert!(image.shares_pixels(&alias));
    assert_eq!(grant.live_bytes().unwrap(), 16);
    assert_eq!(alias.image().data()[0], 19);
    let copy = AdmittedBytes::copy(&grant, CaptureOwner::SourcePixels, image.image().data()).unwrap();
    let second = CaptureImageBuilder::from_bytes(image.size(), FrameLimits::LEGACY, &grant, copy).unwrap().freeze();
    assert!(!image.shares_pixels(&second));
    assert_eq!(grant.live_bytes().unwrap(), 32);
    drop(image);
    assert_eq!(grant.live_bytes().unwrap(), 32);
    drop(alias);
    assert_eq!(grant.live_bytes().unwrap(), 16);
    drop(second);
    assert_eq!(grant.live_bytes().unwrap(), 0);
}
#[test]
fn batches_task_capture_image_allocation_fails_before_uncovered_pixels() {
    let grant = grant(15, 0);
    assert!(CaptureImageBuilder::new(FrameSize::new(2, 2), FrameLimits::LEGACY, &grant, CaptureOwner::SourcePixels).is_err());
    assert_eq!(grant.live_bytes().unwrap(), 0);
    let mut bytes = AdmittedBytes::buffer(&grant, CaptureOwner::SourcePixels, 15).unwrap();
    bytes.extend_from_slice(&[1, 2, 3]).unwrap();
    assert!(bytes.resize(16).is_err());
    assert!(bytes.extend_from_slice(&[0; 13]).is_err());
    assert_eq!(bytes.as_slice(), &[1, 2, 3]);
    assert!(CaptureImageBuilder::from_bytes(FrameSize::new(2, 2), FrameLimits::LEGACY, &grant, bytes).is_err());
    assert_eq!(grant.live_bytes().unwrap(), 0);
}
#[test]
fn batches_task_capture_regions_are_admitted_and_bound_to_the_pixels() {
    let metadata = AdmittedRegions::required_bytes(0, 1).unwrap();
    let grant = grant(16, metadata);
    let mut regions = AdmittedRegions::new(FrameSize::new(2, 2), &grant, 0, 1).unwrap();
    regions.push_dirty(Rect {x: 0, y: 0, width: 1, height: 1}).unwrap();
    assert!(regions.push_dirty(Rect {x: 1, y: 1, width: 1, height: 1}).is_err());
    let mut builder = CaptureImageBuilder::new(FrameSize::new(2, 2), FrameLimits::LEGACY, &grant, CaptureOwner::SourcePixels).unwrap();
    builder.set_regions(regions).unwrap();
    let frozen = builder.freeze();
    let alias = frozen.clone();
    assert_eq!(frozen.image().regions().unwrap().dirty.len(), 1);
    assert_eq!(grant.live_bytes().unwrap(), 16 + metadata);
    drop(frozen);
    assert_eq!(grant.live_bytes().unwrap(), 16 + metadata);
    drop(alias);
    assert_eq!(grant.live_bytes().unwrap(), 0);
}
#[test]
fn batches_task_capture_image_transformations_claim_real_copies_and_keep_aliases() {
    let grant = grant(40, 0);
    let mut builder = CaptureImageBuilder::new(FrameSize::new(2, 2), FrameLimits::LEGACY, &grant, CaptureOwner::SourcePixels).unwrap();
    builder.pixels_mut().copy_from_slice(&[0, 0, 0, 255, 20, 20, 20, 255, 40, 40, 40, 255, 60, 60, 60, 255]);
    let original = builder.freeze();
    let alias = original.downscale(original.size(), FrameLimits::LEGACY, &grant, CaptureOwner::SourcePixels).unwrap();
    assert!(original.shares_pixels(&alias));
    assert_eq!(grant.live_bytes().unwrap(), 16);
    let small = original.downscale(FrameSize::new(1, 1), FrameLimits::LEGACY, &grant, CaptureOwner::SourcePixels).unwrap();
    assert_eq!(small.image().data(), &[30, 30, 30, 255]);
    let rotated = original.rotate(BgraRotation::Clockwise90, FrameLimits::LEGACY, &grant, CaptureOwner::SourcePixels).unwrap();
    assert_eq!(rotated.image().data(), &[40, 40, 40, 255, 0, 0, 0, 255, 60, 60, 60, 255, 20, 20, 20, 255]);
    assert_eq!(grant.live_bytes().unwrap(), 36);
}
#[test]
fn batches_task_capture_bytes_and_regions_reject_another_grant_with_equal_identity() {
    let metadata = AdmittedRegions::required_bytes(0, 1).unwrap();
    let own = grant(16, metadata);
    let foreign = grant(16, metadata);
    let bytes = AdmittedBytes::new(&foreign, CaptureOwner::SourcePixels, 16).unwrap();
    assert!(CaptureImageBuilder::from_bytes(FrameSize::new(2, 2), FrameLimits::LEGACY, &own, bytes).is_err());
    assert_eq!(foreign.live_bytes().unwrap(), 0);
    let mut image = CaptureImageBuilder::new(FrameSize::new(2, 2), FrameLimits::LEGACY, &own, CaptureOwner::SourcePixels).unwrap();
    let regions = AdmittedRegions::new(FrameSize::new(2, 2), &foreign, 0, 1).unwrap();
    assert!(image.set_regions(regions).is_err());
    assert_eq!(foreign.live_bytes().unwrap(), 0);
    assert_eq!(own.live_bytes().unwrap(), 16);
}

#[test]
fn batches_task_capture_library_allocator_runs_after_claim_and_cannot_grow_result() {
    let grant = grant(16, 0);
    let called = std::cell::Cell::new(false);
    let denied = AdmittedBytes::from_allocator(&grant, CaptureOwner::SourcePixels, 17, || {
        called.set(true);
        Ok(vec![0; 17])
    });
    assert!(denied.is_err());
    assert!(!called.get());
    let mut bytes = AdmittedBytes::from_allocator(&grant, CaptureOwner::SourcePixels, 16, || {
        assert_eq!(grant.live_bytes().unwrap(), 16);
        Ok(vec![7; 8])
    }).unwrap();
    assert_eq!(bytes.as_slice(), &[7; 8]);
    let capacity = bytes.capacity() as usize;
    assert!(bytes.resize(capacity + 1).is_err());
    drop(bytes);
    assert_eq!(grant.live_bytes().unwrap(), 0);
}
