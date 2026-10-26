//! Focused source fixtures for the one remote batch suite; never run locally.
use super::*;
use crate::screen::core::{capture_admission::{CaptureAuthority,CaptureGeneration,CapturePlan,SessionAdmission,SourceKey},
    capture_plan::CapturePhases,capture_test_support};
use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};

fn supplement(image:&CaptureGrant)->CapturePlan {
    let mut plan=CapturePlan::new(image.source().clone(),image.generation());
    plan.add_bytes(CaptureOwner::Readback,80,1,CapturePhases::ALL).unwrap();
    plan.add_bytes(CaptureOwner::Metadata,32,1,CapturePhases::ALL).unwrap();
    plan
}
fn referenced(reply:WorkerReply)->WorkerReply {
    WorkerReply::Reference {generation:7,regions:true,limits:FrameLimits::LEGACY,
        family_layout_revision:Some(3),reply:Box::new(reply)}
}

#[test]
fn batches_task_capture_integration_supplement_keeps_image_and_exact_live_owners() {
    let admission=capture_test_support::session(None);
    let source=SourceKey::new(admission.session(),"cursor-image").unwrap();
    let mut plan=CapturePlan::new(source,CaptureGeneration::new(1));
    plan.add_bytes(CaptureOwner::NativeSurface,600*1024*1024,1,CapturePhases::ALL).unwrap();
    let image=admission.authority().approve(plan).unwrap();
    let initial=admission.additional_bytes().unwrap();
    let cursor=admission.authority().approve_supplement(&image,supplement(&image)).unwrap();
    assert_eq!(admission.additional_bytes().unwrap(),initial+112);
    let raw=cursor.claim(CaptureOwner::Readback,80).unwrap();
    let parsed=cursor.claim(CaptureOwner::Metadata,32).unwrap();
    cursor.retire().unwrap();
    assert!(!image.is_retired().unwrap());
    drop(raw);
    assert_eq!(cursor.live_bytes().unwrap(),32);
    assert_eq!(admission.additional_bytes().unwrap(),initial+32);
    drop(cursor);
    assert_eq!(admission.additional_bytes().unwrap(),initial+32);
    drop(parsed);
    assert_eq!(admission.additional_bytes().unwrap(),initial);
    assert!(image.claim(CaptureOwner::NativeSurface,1).is_ok());
}

struct Delegating {
    parent:Arc<SessionAdmission>,
    replacements:Arc<AtomicUsize>,
    supplements:Arc<AtomicUsize>,
}
impl CaptureAuthority for Delegating {
    fn approve(&self,_plan:CapturePlan)->io::Result<CaptureGrant> {
        self.replacements.fetch_add(1,Ordering::SeqCst);
        Err(io::Error::other("fixture rejects image replacement"))
    }
    fn approve_supplement(&self,image:&CaptureGrant,plan:CapturePlan)->io::Result<CaptureGrant> {
        self.supplements.fetch_add(1,Ordering::SeqCst);
        self.parent.authority().approve_supplement(image,plan)
    }
}

#[test]
fn batches_task_capture_integration_delegated_supplement_preserves_member_identity() {
    let parent=capture_test_support::session(None);
    let source=SourceKey::new(parent.session(),"member").unwrap();
    let mut member=CapturePlan::new(source.clone(),CaptureGeneration::new(4));
    member.add_bytes(CaptureOwner::SourcePixels,4,2,CapturePhases::ALL).unwrap();
    let mut plan=CapturePlan::new(SourceKey::new(parent.session(),"family").unwrap(),CaptureGeneration::new(2));
    plan.add_member(member).unwrap();
    let root=parent.authority().approve(plan).unwrap();
    let image=root.for_member(&source,CaptureGeneration::new(4)).unwrap();
    let replacements=Arc::new(AtomicUsize::new(0));
    let supplements=Arc::new(AtomicUsize::new(0));
    let context=SessionAdmission::from_verified_parent(parent.session(),parent.working_bytes(),None,
        Arc::new(Delegating {parent:parent.clone(),replacements:replacements.clone(),supplements:supplements.clone()}),parent.clone()).unwrap();
    let cursor=context.authority().approve_supplement(&image,supplement(&image)).unwrap();
    assert_eq!(replacements.load(Ordering::SeqCst),0);
    assert_eq!(supplements.load(Ordering::SeqCst),1);
    cursor.check_identity(&source,CaptureGeneration::new(4)).unwrap();
    assert!(!root.is_retired().unwrap());
    assert!(image.claim(CaptureOwner::SourcePixels,4).is_ok());
}

#[test]
fn batches_task_capture_integration_supplement_rejects_foreign_identity_and_image_owners() {
    let image=capture_test_support::grant(FrameSize::new(4,2),None);
    for (source,generation,owner) in [
        (SourceKey::new(image.source().session(),"foreign").unwrap(),image.generation(),CaptureOwner::Readback),
        (image.source().clone(),CaptureGeneration::new(99),CaptureOwner::Metadata),
        (image.source().clone(),image.generation(),CaptureOwner::SourcePixels),
        (image.source().clone(),image.generation(),CaptureOwner::NativeSurface),
        (image.source().clone(),image.generation(),CaptureOwner::Transport),
        (image.source().clone(),image.generation(),CaptureOwner::CodecOutput),
    ] {
        let mut plan=CapturePlan::new(source,generation);
        plan.add_bytes(owner,16,1,CapturePhases::ALL).unwrap();
        assert!(image.check_supplement(&plan).is_err());
    }
    let mut import=CapturePlan::new(image.source().clone(),image.generation());
    import.add_bytes(CaptureOwner::Metadata,16,1,CapturePhases::ALL).unwrap();
    import.mark_import().unwrap();
    assert!(image.check_supplement(&import).is_err());
    image.retire().unwrap();
    assert_eq!(image.check_supplement(&supplement(&image)).unwrap_err().kind(),io::ErrorKind::WouldBlock);
}

fn replies(grant:&CaptureGrant,size:FrameSize,raw:bool)->WorkerReply {
    if raw {
        WorkerReply::Raw(CaptureImageBuilder::new(size,FrameLimits::LEGACY,grant,CaptureOwner::SourcePixels).unwrap().freeze())
    } else {
        WorkerReply::Frame(StageFrame::Encoded {
            records:CapturedRecords::from_allocator(grant,48,1,||Ok(vec![vec![9;48]])).unwrap(),
            keyframe:true,capture_ts_us:19,encoder:StageEncoder {name:"tiles".into(),hardware:false},source_size:size})
    }
}

#[test]
fn batches_task_capture_integration_retirement_before_transport_is_a_nonallocating_stale_reply() {
    for raw in [true,false] {
        let size=FrameSize::new(4,2);
        let grant=capture_test_support::grant(size,None);
        let reply=referenced(replies(&grant,size,raw));
        grant.retire().unwrap();
        let live=grant.live_bytes().unwrap();
        assert!(prepare_owned(&reply,23,&grant).unwrap().is_none());
        assert_eq!(grant.live_bytes().unwrap(),live);
        assert!(grant.claim(CaptureOwner::Transport,1).is_err());
        drop(reply);
        assert_eq!(grant.live_bytes().unwrap(),0);
    }
}

#[test]
fn batches_task_capture_integration_prepared_transport_finishes_under_its_original_reference() {
    for raw in [true,false] {
        let size=FrameSize::new(4,2);
        let worker=capture_test_support::grant(size,None);
        let parent=capture_test_support::grant(size,None);
        let reply=referenced(replies(&worker,size,raw));
        let bytes=prepare_owned(&reply,23,&worker).unwrap().unwrap();
        worker.retire().unwrap();
        assert!(worker.live_bytes().unwrap()>=bytes.len() as u64);
        let mut wire=Vec::new();
        send_prepared(&mut wire,&bytes).unwrap();
        let mut input=std::io::Cursor::new(wire);
        let (tag,length)=read_header(&mut input).unwrap().unwrap();
        assert_eq!(tag,OWNED_REFERENCE);
        let (_,grant,payload)=read_owned(&mut input,length,None,|id| {assert_eq!(id,23);Ok(parent.clone())}).unwrap();
        assert_eq!(decode_owned(&payload,&grant,None).unwrap(),reply);
        drop(bytes);
        assert!(worker.live_bytes().unwrap()>0);
        drop(reply);
        assert_eq!(worker.live_bytes().unwrap(),0);
    }
}
