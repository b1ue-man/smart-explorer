//! Parent broker ownership regressions, kept under the original tests module.
use super::*;
use crate::screen::core::{
    capture_admission::{CaptureGeneration, CaptureOwner},
    capture_plan::CapturePhases,
    host_codec_adapters::{HostCodecPlan, HostContract},
    pixel_convert::FrameSize,
};

fn funded() -> (Arc<SessionAdmission>, Arc<CaptureBroker>, u64) {
    let admission = crate::screen::core::capture_test_support::session(None);
    let key = SourceKey::new(admission.session(), "worker").unwrap();
    let broker = CaptureBroker::new(
        admission.clone(),
        SourceKey::new(admission.session(), "synthetic").unwrap(),
    );
    broker.bind_source(key.clone()).unwrap();
    let output = HostOutputPlan::new(
        HostContract::Legacy,
        FrameSize::new(32, 18),
        HostCodecPlan::Tiles,
    )
    .unwrap();
    broker.configure(output.clone()).unwrap();
    let mut plan = CapturePlan::new(key, CaptureGeneration::new(1));
    plan.add_bytes(
        CaptureOwner::NativeSurface,
        600 * 1024 * 1024,
        1,
        CapturePhases::ALL,
    )
    .unwrap();
    plan.add_bgra(
        CaptureOwner::SourcePixels,
        FrameSize::new(32, 18),
        2,
        CapturePhases::ALL,
    )
    .unwrap();
    output
        .for_worker()
        .append_to(&mut plan, HostInputPixels::BorrowedSource)
        .unwrap();
    let id = broker.approve(plan, None).unwrap().id;
    (admission, broker, id)
}

#[test]
fn batches_task_capture_worker_eof_does_not_release_credit_before_process_exit() {
    let (admission, broker, id) = funded();
    let extra = admission.additional_bytes().unwrap();
    assert!(extra > 0);
    broker.reader_ended();
    assert_eq!(admission.additional_bytes().unwrap(), extra);
    assert!(broker.grant(id).is_ok());
    broker.process_exited();
    assert_eq!(admission.additional_bytes().unwrap(), 0);
    assert!(broker.grant(id).is_err());
}

#[test]
fn batches_task_capture_worker_confirmed_exit_keeps_buffered_parent_reply_authority() {
    let (_admission, broker, id) = funded();
    broker.process_exited();
    let buffered = broker.grant(id).unwrap();
    broker.reader_ended();
    assert!(broker.grant(id).is_err());
    assert!(buffered
        .claim(CaptureOwner::CompositionPixels, 32 * 18 * 4)
        .is_ok());
}

#[test]
fn batches_task_capture_worker_retirement_reduction_is_monotonic_until_final_ack() {
    let (admission, broker, id) = funded();
    assert!(admission.additional_bytes().unwrap() > 0);
    let mut owners = [0; OWNER_COUNT];
    owners[CaptureOwner::NativeSurface as usize] = 5 * 1024 * 1024;
    broker.reduced(Reduction { id, owners }).unwrap();
    assert_eq!(admission.additional_bytes().unwrap(), 0);
    let mut enlarged = owners;
    enlarged[CaptureOwner::NativeSurface as usize] += 1;
    assert!(broker
        .reduced(Reduction {
            id,
            owners: enlarged
        })
        .is_err());
    assert!(broker.grant(id).is_ok());
    broker.retired(id).unwrap();
    assert!(broker.grant(id).is_err());
}

#[test]
fn batches_task_capture_integration_cursor_credit_does_not_replace_parent_image() {
    use crate::screen::core::worker_capture_wire::{GrantRequest, GrantValue, PlanWire};
    let (admission, broker, image) = funded();
    let original = broker.grant(image).unwrap();
    let before = admission.additional_bytes().unwrap();
    let mut cursor = CapturePlan::new(original.source().clone(), original.generation());
    cursor
        .add_bytes(CaptureOwner::Readback, 80, 1, CapturePhases::ALL)
        .unwrap();
    cursor
        .add_bytes(CaptureOwner::Metadata, 32, 1, CapturePhases::ALL)
        .unwrap();
    let request = GrantRequest::SupplementPlan {
        request: 17,
        image,
        plan: PlanWire::new(&cursor),
    };
    let payload = crate::screen::core::wire::to_json(&request).unwrap();
    let request = crate::screen::core::wire::from_json(&payload).unwrap();
    let (request, credit) = broker.resource(request).unwrap();
    assert_eq!(request, 17);
    let GrantValue::Supplement(credit) = credit.unwrap() else {
        panic!("wrong cursor credit class");
    };
    assert_eq!(admission.additional_bytes().unwrap(), before + 112);
    assert_eq!(
        broker.current().unwrap().book_identity(),
        original.book_identity()
    );
    assert!(
        broker.grant(credit.id).is_err(),
        "cursor has no Parent image partition"
    );
    assert!(original.claim(CaptureOwner::Transport, 64).is_ok());
    broker.retired(credit.id).unwrap();
    assert_eq!(admission.additional_bytes().unwrap(), before);
    assert!(broker
        .grant(image)
        .unwrap()
        .claim(CaptureOwner::Transport, 64)
        .is_ok());
    assert_eq!(
        broker.current().unwrap().book_identity(),
        original.book_identity()
    );
}

#[test]
fn batches_task_capture_integration_broker_checks_cursor_member_whitelist_and_generation() {
    use crate::screen::core::worker_capture_wire::{GrantRequest, PlanWire};
    let (_, broker, image) = funded();
    let original = broker.grant(image).unwrap();
    for (source, generation, owner) in [
        (
            SourceKey::new(original.source().session(), "foreign-member").unwrap(),
            original.generation(),
            CaptureOwner::Readback,
        ),
        (
            original.source().clone(),
            CaptureGeneration::new(99),
            CaptureOwner::Metadata,
        ),
        (
            original.source().clone(),
            original.generation(),
            CaptureOwner::CodecOutput,
        ),
    ] {
        let mut cursor = CapturePlan::new(source, generation);
        cursor.add_bytes(owner, 32, 1, CapturePhases::ALL).unwrap();
        let (_, result) = broker
            .resource(GrantRequest::SupplementPlan {
                request: 18,
                image,
                plan: PlanWire::new(&cursor),
            })
            .unwrap();
        assert!(result.is_err());
    }
    assert!(original.claim(CaptureOwner::Transport, 64).is_ok());
    assert_eq!(
        broker.current().unwrap().book_identity(),
        original.book_identity()
    );
}

#[test]
fn batches_task_capture_integration_broker_admits_the_bound_family_member_cursor() {
    use crate::screen::core::worker_capture_wire::{GrantRequest, GrantValue, PlanWire};
    let admission = crate::screen::core::capture_test_support::session(None);
    let root = SourceKey::new(admission.session(), "family").unwrap();
    let member = SourceKey::new(admission.session(), "x11-member").unwrap();
    let broker = CaptureBroker::new(
        admission.clone(),
        SourceKey::new(admission.session(), "synthetic").unwrap(),
    );
    broker.bind_source(root.clone()).unwrap();
    let output = HostOutputPlan::new(
        HostContract::Legacy,
        FrameSize::new(4, 2),
        HostCodecPlan::Tiles,
    )
    .unwrap();
    broker.configure(output.clone()).unwrap();
    let mut plan = CapturePlan::new(root, CaptureGeneration::new(2));
    plan.add_bgra(
        CaptureOwner::CompositionPixels,
        FrameSize::new(4, 2),
        2,
        CapturePhases::ALL,
    )
    .unwrap();
    let mut child = CapturePlan::new(member.clone(), CaptureGeneration::new(4));
    child
        .add_bgra(
            CaptureOwner::SourcePixels,
            FrameSize::new(4, 2),
            2,
            CapturePhases::ALL,
        )
        .unwrap();
    plan.add_member(child).unwrap();
    output
        .for_worker()
        .append_to(&mut plan, HostInputPixels::BorrowedSource)
        .unwrap();
    let image = broker.approve(plan, None).unwrap();
    let parent = broker.current().unwrap();
    let mut cursor = CapturePlan::new(member, CaptureGeneration::new(4));
    cursor
        .add_bytes(CaptureOwner::Readback, 64, 1, CapturePhases::ALL)
        .unwrap();
    cursor
        .add_bytes(CaptureOwner::Metadata, 8, 1, CapturePhases::ALL)
        .unwrap();
    let (_, result) = broker
        .resource(GrantRequest::SupplementPlan {
            request: 19,
            image: image.id,
            plan: PlanWire::new(&cursor),
        })
        .unwrap();
    let GrantValue::Supplement(cursor) = result.unwrap() else {
        panic!("wrong family cursor credit class");
    };
    assert_eq!(cursor.source, "x11-member");
    assert_eq!(cursor.generation, 4);
    broker.retired(cursor.id).unwrap();
    assert_eq!(
        broker.current().unwrap().book_identity(),
        parent.book_identity()
    );
    assert!(parent.claim(CaptureOwner::CompositionPixels, 32).is_ok());
}

#[test]
fn batches_task_capture_integration_new_plan_keeps_an_old_prepared_reply_receivable() {
    use crate::screen::core::{
        capture_image::CaptureImageBuilder, capture_transport, frame_budget::FrameLimits,
        worker_wire::WorkerReply,
    };
    let (_, broker, image) = funded();
    let old = broker.state.lock().unwrap().entries[&image]
        .worker
        .clone()
        .unwrap();
    let size = FrameSize::new(32, 18);
    let reply = WorkerReply::Reference {
        generation: 7,
        regions: true,
        limits: FrameLimits::LEGACY,
        family_layout_revision: None,
        reply: Box::new(WorkerReply::Raw(
            CaptureImageBuilder::new(size, FrameLimits::LEGACY, &old, CaptureOwner::SourcePixels)
                .unwrap()
                .freeze(),
        )),
    };
    let prepared = capture_transport::prepare_owned(&reply, image, &old)
        .unwrap()
        .unwrap();
    let mut next = CapturePlan::new(old.source().clone(), CaptureGeneration::new(2));
    next.add_bgra(CaptureOwner::SourcePixels, size, 2, CapturePhases::ALL)
        .unwrap();
    let output = HostOutputPlan::new(HostContract::Legacy, size, HostCodecPlan::Tiles).unwrap();
    output
        .for_worker()
        .append_to(&mut next, HostInputPixels::BorrowedSource)
        .unwrap();
    let next = broker.approve(next, None).unwrap();
    let mut wire = Vec::new();
    capture_transport::send_prepared(&mut wire, &prepared).unwrap();
    let mut wire = std::io::Cursor::new(wire);
    let (_, length) = capture_transport::read_header(&mut wire).unwrap().unwrap();
    let (_, parent, payload) =
        capture_transport::read_owned(&mut wire, length, None, |id| broker.grant(id)).unwrap();
    let decoded = capture_transport::decode_owned(&payload, &parent, None).unwrap();
    assert_eq!(decoded, reply);
    drop(prepared);
    drop(reply);
    broker.retired(image).unwrap();
    assert!(!parent.is_retired().unwrap());
    assert!(
        parent.claim(CaptureOwner::Transport, 64).is_ok(),
        "received Parent results keep their downstream plan"
    );
    assert!(matches!(
        &decoded,
        WorkerReply::Reference { generation: 7, .. }
    ));
    let WorkerReply::Reference {
        reply: decoded_image,
        ..
    } = &decoded
    else {
        panic!("missing received reference");
    };
    let WorkerReply::Raw(decoded_image) = decoded_image.as_ref() else {
        panic!("missing received RAW image");
    };
    use crate::screen::core::host_backend::ScreenEncoder;
    let mut encoder = crate::screen::core::host_codec_adapters::AdmittedTilesEncoder::new(
        size,
        FrameLimits::LEGACY,
        decoded_image.grant(),
    )
    .unwrap();
    let request = crate::screen::core::host_stage::StageRequest {
        frame_contract: None,
        visible_rects: None,
        frame_id: 2,
        keyframe: true,
        target_bps: 1,
        size,
        choice: crate::screen::core::negotiate::CodecChoice {
            codec: crate::screen::core::negotiate::CodecKind::Tiles,
            profile: None,
            encoder_hardware: false,
            decoder_hardware: false,
            max_size: size,
            max_pixels: size.pixels(),
            max_macroblocks_per_second: None,
        },
        encoder_epoch: 2,
        fps: 1,
    };
    assert!(
        !encoder
            .encode_captured(decoded_image, &request)
            .unwrap()
            .records
            .is_empty(),
        "a received RAW snapshot still encodes after the Worker retired its credit"
    );
    assert_eq!(
        broker.current().unwrap().generation().value(),
        next.generation
    );
    assert!(broker
        .grant(next.id)
        .unwrap()
        .claim(CaptureOwner::Transport, 64)
        .is_ok());
}
