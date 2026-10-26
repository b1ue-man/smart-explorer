//! C7 fixtures for the single final remote suite, not a local entrypoint.
use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
use super::{capture_admission::{CaptureGeneration,CaptureOwner,CapturePlan,SourceKey},
    capture_import::CaptureImportNeed,capture_plan::CapturePhases,
    frame_memory::{CapacityCeiling,MemoryPool},frame_budget::FrameLimits,
    capture_image::CaptureImageBuilder,capture_bytes::AdmittedBytes,pixel_convert::FrameSize};
#[test]
fn batches_task_capture_import_unbound_is_exact_and_is_not_image_authority() {
    let pool=Arc::new(MemoryPool::default());
    let session=pool.admit_session(None,CapacityCeiling::bounded(4*1024*1024),1024*1024,"imports").unwrap();
    let source=SourceKey::new("imports","monitor:portal").unwrap();
    assert!(session.import_connection(&source).is_err());
    session.register_prepared_source(source.clone()).unwrap();
    let connection=session.import_connection(&source).unwrap();
    let need=CaptureImportNeed::new(source.clone(),connection.id(1).unwrap(),1024*1024,128).unwrap();
    let grant=session.authority().approve_import(need.clone()).unwrap();
    assert_eq!(session.working_bytes(),1024*1024);
    assert_eq!(session.additional_bytes().unwrap(),128);
    grant.check_identity(&source,&connection.id(1).unwrap()).unwrap();
    let mapped=grant.claim_mapping().unwrap();
    assert!(grant.claim_mapping().is_err());
    let allocation=Arc::new(AtomicBool::new(false));let attempted=allocation.clone();
    assert!(AdmittedBytes::from_allocator(&mapped.grant().unwrap(),CaptureOwner::Metadata,4,move|| {
        attempted.store(true,Ordering::SeqCst);Ok(vec![0;4])
    }).is_err());
    assert!(!allocation.load(Ordering::SeqCst));
    assert!(CaptureImageBuilder::new(FrameSize::new(1,1),FrameLimits::LEGACY,&mapped.grant().unwrap(),CaptureOwner::NativeSurface).is_err());
    let mut frame=CapturePlan::new(source.clone(),CaptureGeneration::new(1));
    frame.add_bytes(CaptureOwner::SourcePixels,4,1,CapturePhases::ALL).unwrap();
    assert!(session.authority().approve(frame).is_err());
    assert!(session.authority().approve_import(need).is_err());
    let stale_id = connection.id(2).unwrap();
    drop(connection);
    let stale=CaptureImportNeed::new(source,stale_id,4096,0).unwrap();
    assert!(session.authority().approve_import(stale).is_err());
    drop(grant);
    assert_eq!(session.additional_bytes().unwrap(),128);
    drop(mapped);
    assert_eq!(session.additional_bytes().unwrap(),0);
}
#[test]
fn batches_task_capture_import_old_connection_cannot_close_a_new_connection() {
    let pool=Arc::new(MemoryPool::default());
    let session=pool.admit_session(None,CapacityCeiling::bounded(1024*1024),1024*1024,"imports").unwrap();
    let source=SourceKey::new("imports","monitor:portal").unwrap();
    session.register_prepared_source(source.clone()).unwrap();
    let old=session.import_connection(&source).unwrap();
    let current=session.import_connection(&source).unwrap();
    assert_ne!(old.generation(),current.generation());
    let stale=CaptureImportNeed::new(source.clone(),old.id(1).unwrap(),4096,0).unwrap();
    assert!(session.authority().approve_import(stale).is_err());drop(old);
    let current_need=CaptureImportNeed::new(source,current.id(1).unwrap(),4096,0).unwrap();
    assert!(session.authority().approve_import(current_need).is_ok());
}
