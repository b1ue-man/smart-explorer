// Smart Explorer P28b/V3 patch (2026-10-25); see vendor/SMART_EXPLORER_ROUTE_POLICY.md.
//! Source cases of the policy module (moved out unchanged, A7 2026-11-06).

use bytes::Bytes;
use super::*;
use crate::{FrameStats, VarInt, frame, connection::{PacketBuilder, TransmitBuf, datagrams::DatagramState}};

#[test]
fn route_policy_task_serializer_blocks_payload_and_redacts_close() {
    let mut bytes = Vec::new();
    let mut transmit = TransmitBuf::new(&mut bytes, std::num::NonZeroUsize::MIN, 1200);
    transmit.start_new_datagram();
    let mut builder = PacketBuilder::policy_data_buf(&mut transmit, false);
    let mut stats = FrameStats::default();
    builder.write_frame(frame::Datagram { data: Bytes::from_static(b"private") }, &mut stats);
    builder.write_frame(frame::StreamMeta {
        id: crate::StreamId(0), offsets: 0..7, fin: false,
    }.encoder(true), &mut stats);
    assert_eq!(builder.buf.len(), 0);
    builder.write_frame(frame::Ping, &mut stats);
    let close = frame::Close::Application(crate::ApplicationClose {
        error_code: VarInt::from_u32(7), reason: Bytes::from_static(b"private close reason"),
    });
    builder.write_frame(close.encoder(256), &mut stats);
    drop(builder);
    let mut frames = frame::Iter::new(Bytes::copy_from_slice(transmit.as_mut_slice())).unwrap();
    assert!(matches!(frames.next().unwrap().unwrap(), frame::Frame::Ping));
    assert!(matches!(frames.next().unwrap().unwrap(),
        frame::Frame::Close(frame::Close::Application(close))
            if close.reason.is_empty() && close.error_code == VarInt::from_u32(7)));
    assert!(frames.next().is_none());
}

#[test]
fn route_policy_task_datagram_queue_survives_block_and_grant() {
    let mut bytes = Vec::new();
    let mut transmit = TransmitBuf::new(&mut bytes, std::num::NonZeroUsize::MIN, 1200);
    transmit.start_new_datagram();
    let mut datagrams = DatagramState::default();
    datagrams.outgoing.push_back(frame::Datagram { data: Bytes::from_static(b"payload") });
    datagrams.outgoing_total = 7;
    let mut stats = FrameStats::default();
    let mut blocked = PacketBuilder::policy_data_buf(&mut transmit, false);
    assert!(!datagrams.write(&mut blocked, &mut stats));
    assert_eq!(datagrams.outgoing_total, 7);
    assert_eq!(datagrams.outgoing.len(), 1);
    drop(blocked);
    let mut allowed = PacketBuilder::policy_data_buf(&mut transmit, true);
    assert!(datagrams.write(&mut allowed, &mut stats));
    assert_eq!(datagrams.outgoing_total, 0);
    assert!(datagrams.outgoing.is_empty());
    drop(allowed);
    let frame = frame::Iter::new(Bytes::copy_from_slice(transmit.as_mut_slice()))
        .unwrap().next().unwrap().unwrap();
    assert!(matches!(frame, frame::Frame::Datagram(frame) if frame.data.as_ref() == b"payload"));
}
