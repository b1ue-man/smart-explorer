//! Private picture transport must adopt bytes only after a matching credit.
use super::{
    capture_records::CapturedRecords,
    capture_test_support, capture_transport,
    frame_budget::FrameLimits,
    host_stage::{StageEncoder, StageFrame},
    pixel_convert::{BgraImage, FrameSize},
    worker_wire::WorkerReply,
};
use std::io::Cursor;

fn referenced(reply: WorkerReply) -> WorkerReply {
    WorkerReply::Reference {
        generation: 7,
        regions: true,
        limits: FrameLimits::LEGACY,
        family_layout_revision: Some(3),
        reply: Box::new(reply),
    }
}

#[test]
fn batches_task_capture_worker_raw_and_records_roundtrip_under_the_original_credit() {
    let size = FrameSize::new(32, 18);
    let grant = capture_test_support::grant(size, None);
    let mut pixels = BgraImage::new(size).unwrap();
    pixels.data_mut().fill(31);
    let raw = WorkerReply::Raw(capture_test_support::image(&pixels, &grant));
    assert!(raw.encode().is_err());
    let records = CapturedRecords::from_allocator(&grant, 48, 1, || Ok(vec![vec![9; 48]])).unwrap();
    let encoded = WorkerReply::Frame(StageFrame::Encoded {
        records,
        keyframe: true,
        capture_ts_us: 19,
        encoder: StageEncoder {
            name: "tiles".into(),
            hardware: false,
        },
        source_size: size,
    });
    assert!(encoded.encode().is_err());
    for reply in [referenced(raw), referenced(encoded)] {
        let mut wire = Vec::new();
        capture_transport::send_owned(&mut wire, &reply, 23, &grant).unwrap();
        let mut input = Cursor::new(wire);
        let (tag, length) = capture_transport::read_header(&mut input).unwrap().unwrap();
        assert_eq!(tag, super::worker_capture_wire::OWNED_REFERENCE);
        let (id, owner, payload) = capture_transport::read_owned(&mut input, length, None, |id| {
            assert_eq!(id, 23);
            Ok(grant.clone())
        })
        .unwrap();
        assert_eq!(id, 23);
        assert_eq!(owner.book_identity(), grant.book_identity());
        assert_eq!(
            capture_transport::decode_owned(&payload, &owner, None).unwrap(),
            reply
        );
    }
}

#[test]
fn batches_task_capture_worker_wrong_credit_is_refused_before_payload_read() {
    let size = FrameSize::new(32, 18);
    let grant = capture_test_support::grant(size, None);
    let image = capture_test_support::image(&BgraImage::new(size).unwrap(), &grant);
    let mut wire = Vec::new();
    capture_transport::send_owned(&mut wire, &referenced(WorkerReply::Raw(image)), 23, &grant)
        .unwrap();
    let mut input = Cursor::new(wire);
    let (_, length) = capture_transport::read_header(&mut input).unwrap().unwrap();
    assert!(
        capture_transport::read_owned(&mut input, length, None, |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "unknown grant",
            ))
        })
        .is_err()
    );
    assert_eq!(input.position(), 5 + 58);
}
