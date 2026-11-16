//! Task suite „Bildkette“ BK3b (copy 15 of the BK3 table): the daemon reads
//! an encoded picture of the host process straight into its records and
//! gets the same answer as through the transport buffer, for still images
//! as well; a cut-off picture is an error, never a shorter record. Not
//! registered yet (`docs/tests/register.tsv`): the manager adds the rows.
use super::*;
use crate::screen::core::capture_image::CaptureImageBuilder;
use crate::screen::core::capture_test_support;
use crate::screen::core::worker_capture_wire::OWNED_REFERENCE;

fn reply(grant: &CaptureGrant, size: FrameSize, raw: bool) -> WorkerReply {
    let inner = if raw {
        WorkerReply::Raw(
            CaptureImageBuilder::new(size, FrameLimits::LEGACY, grant, CaptureOwner::SourcePixels)
                .unwrap()
                .freeze(),
        )
    } else {
        let records = CapturedRecords::from_allocator(grant, 96, 3, || {
            Ok(vec![vec![1; 48], vec![], vec![7; 48]])
        })
        .unwrap();
        WorkerReply::Frame(StageFrame::Encoded {
            records,
            keyframe: true,
            capture_ts_us: 19,
            encoder: StageEncoder {
                name: "MFT [GPU input (D3D11)]".into(),
                hardware: true,
            },
            source_size: size,
        })
    };
    WorkerReply::Reference {
        generation: 7,
        regions: true,
        limits: FrameLimits::LEGACY,
        family_layout_revision: Some(3),
        reply: Box::new(inner),
    }
}

#[test]
fn bildkette_task_chain_records_come_from_the_pipe_without_a_transport_copy() {
    for raw in [false, true] {
        let size = FrameSize::new(4, 2);
        let worker = capture_test_support::grant(size, None);
        let parent = capture_test_support::grant(size, None);
        let sent = reply(&worker, size, raw);
        let bytes = prepare_owned(&sent, 23, &worker).unwrap().unwrap();
        let mut wire = Vec::new();
        send_prepared(&mut wire, &bytes).unwrap();
        // The reference: transport buffer, then decode.
        let mut input = std::io::Cursor::new(wire.clone());
        let (tag, length) = read_header(&mut input).unwrap().unwrap();
        assert_eq!(tag, OWNED_REFERENCE);
        let (_, grant, payload) =
            read_owned(&mut input, length, None, |_| Ok(parent.clone())).unwrap();
        let reference = decode_owned(&payload, &grant, None).unwrap();
        assert_eq!(reference, sent, "raw {raw}");
        // Both answers hold their own claims: one at a time.
        drop((reference, payload));
        // Straight into the records.
        let mut input = std::io::Cursor::new(wire.clone());
        let (_, length) = read_header(&mut input).unwrap().unwrap();
        let direct = read_owned_reply(&mut input, length, None, |id| {
            assert_eq!(id, 23);
            Ok(parent.clone())
        })
        .unwrap();
        assert_eq!(direct, sent, "raw {raw}");
        drop(direct);
        assert_eq!(input.position() as usize, wire.len(), "nothing left unread");
        // A picture cut off in its last record is an error.
        let mut cut = std::io::Cursor::new(wire[..wire.len() - 1].to_vec());
        let (_, length) = read_header(&mut cut).unwrap().unwrap();
        assert!(read_owned_reply(&mut cut, length, None, |_| Ok(parent.clone())).is_err());
    }
}
