use std::future::Future;
use std::time::Duration;

use super::*;
use crate::screen::core::frame_budget::{
    FrameCapabilities, FrameContract, MAX_SESSION_WORKING_BYTES,
};
use crate::screen::core::gop_stream::PictureEvent;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::video_tcp_wire::{link_record_limit, CONFIRM, DEFAULT_LINK_RECORD_BYTES};
use crate::screen::core::video_transport::video_delivery::MessageDelivery;
use crate::screen::core::video_transport::video_message::{encode, Admission, PictureMeta};
use crate::screen::core::wire_binary::{GopHeader, StreamKind};
use crate::screen::core::wire_control::FrameAck;

fn run<F: Future>(work: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(work)
}

/// An extended contract of 4096×2304 (the largest working memory a session
/// admits) for lossless tiles without hybrid regions.
fn tile_contract() -> FrameContract {
    let caps =
        FrameCapabilities::admitted(FrameSize::new(4096, 2304), MAX_SESSION_WORKING_BYTES).unwrap();
    let mut contract = FrameContract::negotiate(Some(caps), Some(caps))
        .unwrap()
        .unwrap();
    contract.hybrid_regions = false;
    contract
}

/// The message of one tile picture of `header`'s group: the group's key
/// frame when it has no picture before it (`prev` 0).
fn tile_message(header: &GopHeader, frame_id: u64, prev: u64, records: &[Vec<u8>]) -> Vec<u8> {
    let meta = PictureMeta {
        frame_id,
        capture_ts_us: frame_id * 1_000,
        keyframe: prev == 0,
        continuation: false,
        prev_frame_id: prev,
    };
    encode(header, meta, records).unwrap()
}

/// User 2026-11-25 („TCP und UDP, DIREKT“, „was du anzeigst MUSS
/// funktionieren“): under „TCP“ a lossless tile picture larger than one
/// picture record (the link's bound before, 16 MiB and 4 KiB) goes from the
/// host's record carrier over a direct TCP link (two sockets on loopback,
/// TLS with the pinned key and the token) as one record, the viewer's
/// reader takes it within its contract's bound and passes it on whole, and
/// the delivery hands its records to the decoder as a picture stream does,
/// then `FRAME_ACK`; a small picture follows on the same link, and a clean
/// end of the host reads as the link's end.
#[test]
fn video_tcp_task_tile_picture_beyond_one_picture_record_goes_whole_over_the_link() {
    let contract = tile_contract();
    let header = GopHeader {
        gop_id: 7,
        frame_contract: Some(contract),
        family_layout_revision: None,
        kind: StreamKind::Video,
        codec: CodecKind::Tiles,
        profile: None,
        size: FrameSize::new(1920, 1080),
    };
    // Full records of the tile codec, as many as one old link record could
    // not carry.
    let record_len = header.record_limit();
    let count = DEFAULT_LINK_RECORD_BYTES / record_len + 1;
    let records: Vec<Vec<u8>> = (0..count)
        .map(|n| (0..record_len).map(|i| ((i + n) % 251) as u8).collect())
        .collect();
    let large = tile_message(&header, 41, 0, &records);
    let small_records = vec![vec![0x5a; 300], vec![0xa5; 200]];
    let small = tile_message(&header, 42, 41, &small_records);
    let admission = Admission {
        contract: Some(contract),
        local: Some(contract.limits),
        memory_bytes: None,
    };
    let bound = admission.max_message().unwrap();
    assert!(large.len() > DEFAULT_LINK_RECORD_BYTES, "{}", large.len());
    assert!(large.len() <= bound);
    assert_eq!(link_record_limit(Some(contract)).unwrap(), bound);

    run(async move {
        let host = HostTcpLocal::loopback().unwrap();
        let viewer = ViewerTcpLocal::loopback().unwrap();
        let (answer, offer) = (host.answer(), viewer.offer());
        let deadline = Instant::now() + Duration::from_secs(5);
        let host = tokio::spawn(async move { host.establish(&offer, deadline).await });
        let viewer = tokio::spawn(async move { viewer.establish(&answer, deadline).await });
        let (host, viewer) = (host.await.unwrap().unwrap(), viewer.await.unwrap().unwrap());
        let TcpPictureLink {
            reader: mut viewer_reader,
            writer: _viewer_writer,
            ..
        } = viewer;
        let TcpPictureLink {
            reader: _host_reader,
            writer: host_writer,
            ..
        } = host;

        // As `ViewerTcp::answer`: the reader takes the contract's largest
        // message and passes every record on.
        viewer_reader.set_max_record(bound);
        let (records_tx, mut records_rx) = mpsc::channel::<Vec<u8>>(4);
        let forward = tokio::spawn(async move {
            let end = forward_pictures(&mut viewer_reader, &records_tx).await;
            (end, viewer_reader.bytes_received())
        });

        // As `spawn_host_tcp`: the host's writer as the record carrier.
        let sender = TcpRecordSender {
            writer: Mutex::new(host_writer),
            link: "lan",
        };
        let (sent_large, sent_small) = (large.clone(), small.clone());
        let sending = tokio::spawn(async move {
            sender.send(&sent_large).await.unwrap();
            sender.send(&sent_small).await.unwrap();
            sender
        });
        let received = records_rx.recv().await.unwrap();
        assert_eq!(received.len(), large.len());
        assert!(received == large, "the large picture arrived changed");

        let (events_tx, mut events_rx) = mpsc::channel::<PictureEvent>(count + 2);
        let (acks_tx, mut acks_rx) = mpsc::channel::<ViewerFrame>(4);
        let mut delivery = MessageDelivery::new(events_tx, acks_tx, admission);
        assert!(delivery.deliver(&received, None, 5).await);
        for (index, payload) in records.iter().enumerate() {
            let Some(PictureEvent::Record {
                header: got,
                record,
                received_us,
            }) = events_rx.recv().await
            else {
                panic!("record {index} of the large picture did not reach the decoder");
            };
            assert_eq!(got, header);
            assert_eq!((record.frame_id, received_us), (41, 5));
            assert!(record.is_keyframe());
            assert_eq!(record.has_more(), index + 1 < count);
            assert!(&record.payload == payload, "record {index} changed");
        }
        let ack = |frame_id| {
            ViewerFrame::FrameAck(FrameAck {
                gop_id: 7,
                frame_id,
                received_us: 5,
            })
        };
        assert_eq!(acks_rx.recv().await.unwrap(), ack(41));

        // The next picture on the same link continues the chain.
        assert_eq!(records_rx.recv().await.unwrap(), small);
        assert!(delivery.deliver(&small, None, 5).await);
        for payload in &small_records {
            let Some(PictureEvent::Record { record, .. }) = events_rx.recv().await else {
                panic!("the small picture did not reach the decoder");
            };
            assert_eq!((record.frame_id, &record.payload), (42, payload));
        }
        assert_eq!(acks_rx.recv().await.unwrap(), ack(42));

        // A clean end of the host ends the forwarding with its reason, after
        // every byte the host wrote (its token confirmation included).
        let sender = sending.await.unwrap();
        let writer = sender.writer.into_inner();
        let written = writer.bytes_sent();
        assert_eq!(
            written,
            (CONFIRM.len() + 4 + large.len() + 4 + small.len()) as u64
        );
        writer.finish().await.unwrap();
        let (end, read) = forward.await.unwrap();
        assert_eq!(end.as_deref(), Some("direct TCP link ended by the host"));
        assert_eq!(read, written);
    });
}
