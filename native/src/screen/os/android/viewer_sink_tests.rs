//! The queue behind `screenPull` and the picture-group logic in front of it
//! (SCR-M9, U28-4) without a device: the layout and the codes of a pull, the
//! rule for a buffer that is too small, the stall rule with an injected
//! clock, and the tile pictures in the byte order of Android bitmaps.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::viewer_sink::*;
use crate::screen::core::host_backend::{ScreenEncoder, TilesScreenEncoder};
use crate::screen::core::negotiate::{CodecKind, H264Profile};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::session_state::KeyframeRequester;
use crate::screen::core::wire_binary::{
    FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME, FLAG_MORE,
};
use crate::screen::core::wire_control::{KeyframeReason, KeyframeRequest};

fn header(gop_id: u64, codec: CodecKind, size: FrameSize) -> GopHeader {
    GopHeader {
        gop_id,
        kind: StreamKind::Video,
        codec,
        profile: (codec == CodecKind::H264).then_some(H264Profile::High),
        size,
    }
}

fn record(frame_id: u64, ts: u64, key: bool, payload: &[u8]) -> FrameRecord {
    FrameRecord {
        frame_id,
        capture_ts_us: ts,
        flags: if key { FLAG_KEYFRAME } else { 0 },
        payload: payload.to_vec(),
    }
}

fn sink() -> (ViewerSink, Arc<PullQueue>) {
    let queue = Arc::new(PullQueue::default());
    let keyframes = Arc::new(Mutex::new(KeyframeRequester::default()));
    (ViewerSink::new(queue.clone(), keyframes), queue)
}

fn pull(queue: &PullQueue, capacity: usize) -> (i32, Vec<u8>) {
    let mut dst = vec![0xAA; capacity];
    let n = queue.pull_into(&mut dst, Duration::from_millis(20));
    (n, dst)
}

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

#[test]
fn screen_task_android_pull_layout() {
    let queue = PullQueue::default();
    let flags = FLAG_KEYFRAME | FLAG_NEW_STREAM | FLAG_LAST_OF_PICTURE | (7 << GOP_SHIFT);
    let pushed = queue.push(0x0102_0304_0506_0708, flags, vec![1, 2, 3], 10);
    assert_eq!(
        pushed,
        Pushed {
            queued: true,
            stalled: false
        }
    );
    let (n, dst) = pull(&queue, 64);
    assert_eq!(n, 19);
    assert_eq!(dst[0..8], [8, 7, 6, 5, 4, 3, 2, 1], "pts_us little endian");
    assert_eq!(word(&dst, 8), flags);
    assert_eq!(dst[8..12], flags.to_le_bytes());
    assert_eq!(dst[12..16], [3, 0, 0, 0], "len little endian");
    assert_eq!(dst[16..19], [1, 2, 3]);
    assert_eq!(dst[19], 0xAA, "nothing beyond the payload is written");
    // Empty: timeout, nothing written; closed: ended.
    let (n, dst) = pull(&queue, 32);
    assert_eq!(n, PULL_TIMEOUT);
    assert!(dst.iter().all(|byte| *byte == 0xAA));
    queue.close();
    assert_eq!(pull(&queue, 32).0, PULL_ENDED);
    assert!(!queue.push(0, 0, vec![1], 1).queued);
    // The codes of the contract.
    assert_eq!(
        [PULL_TIMEOUT, PULL_ENDED, PULL_TOO_SMALL, PULL_BRIDGE_ERROR],
        [-1, -2, -3, -4]
    );
    assert_eq!(MAX_PULL_WAIT, Duration::from_millis(1000));
}

#[test]
fn screen_task_android_viewer_pull_buffer_too_small() {
    let queue = PullQueue::default();
    let payload = vec![9u8; 100];
    queue.push(5, FLAG_KEYFRAME, payload.clone(), 0);
    // A buffer that holds the head but not the payload: −3, the length is named.
    let (n, dst) = pull(&queue, 50);
    assert_eq!(n, PULL_TOO_SMALL);
    assert_eq!(word(&dst, 12), 100, "the payload length the app needs");
    assert!(dst[0..12].iter().all(|byte| *byte == 0xAA));
    assert!(
        dst[16..].iter().all(|byte| *byte == 0xAA),
        "no payload copied"
    );
    // Not even the head fits: −3 and nothing written.
    let (n, dst) = pull(&queue, 8);
    assert_eq!(n, PULL_TOO_SMALL);
    assert!(dst.iter().all(|byte| *byte == 0xAA));
    // The picture stayed queued.
    assert_eq!(queue.len(), 1);
    let (n, dst) = pull(&queue, 16 + 100);
    assert_eq!(n, 116);
    assert_eq!(dst[16..], payload[..]);
    assert_eq!(queue.len(), 0);
}

#[test]
fn screen_task_android_pull_stall_resets() {
    let (mut sink, queue) = sink();
    let size = FrameSize::new(1280, 720);
    let first = header(1, CodecKind::H264, size);
    // A key frame and a delta frame wait for an app that does not pull.
    let out = sink.record(&first, &record(1, 10, true, b"idr"), 0);
    assert!(matches!(
        out.as_slice(),
        [SinkOutput::NewStream { gop: 1, .. }]
    ));
    assert!(sink
        .record(&first, &record(2, 20, false, b"p1"), 100)
        .is_empty());
    assert_eq!(queue.len(), 2);
    // The oldest picture is older than STALL_US: emptied, one Reset request.
    let out = sink.record(&first, &record(3, 30, false, b"p2"), STALL_US + 101);
    assert_eq!(queue.len(), 0);
    assert_eq!(
        out,
        vec![SinkOutput::Keyframe(KeyframeRequest {
            reason: KeyframeReason::Reset,
            gop_id: Some(1)
        })]
    );
    // Until the new key frame the app gets nothing.
    assert!(sink
        .record(&first, &record(4, 40, false, b"p3"), STALL_US + 200)
        .is_empty());
    assert_eq!(pull(&queue, 64).0, PULL_TIMEOUT);
    // The key frame of a new group starts the picture again, marked as a
    // group start for the app; the request is answered.
    let second = header(2, CodecKind::H264, size);
    let out = sink.record(&second, &record(5, 50, true, b"idr2"), STALL_US + 300);
    assert!(matches!(
        out.as_slice(),
        [SinkOutput::NewStream { gop: 2, .. }]
    ));
    let (n, dst) = pull(&queue, 64);
    assert_eq!(n, 16 + 4);
    let flags = word(&dst, 8);
    assert_eq!(flags & FLAG_KEYFRAME, FLAG_KEYFRAME);
    assert_eq!(flags & FLAG_NEW_STREAM, FLAG_NEW_STREAM);
    assert_eq!(flags >> GOP_SHIFT, 2);
    assert_eq!(dst[16..20], *b"idr2");
}

#[test]
fn screen_task_android_sink_groups_and_tiles() {
    let (mut sink, queue) = sink();
    let size = FrameSize::new(64, 48);
    let mut pixels = vec![0u8; size.bgra_len()];
    pixels[..4].copy_from_slice(&[10, 20, 30, 255]);
    let mut encoder = TilesScreenEncoder::new(size).unwrap();
    let tiles = header(3, CodecKind::Tiles, size);
    let mut feed = |sink: &mut ViewerSink, pixels: &[u8], key: bool, now: u64| {
        let frame = BgraImage::from_vec(size, pixels.to_vec()).unwrap();
        let encoded = encoder.encode(&frame, key, 0).unwrap();
        let count = encoded.records.len();
        let mut out = Vec::new();
        for (index, bytes) in encoded.records.iter().enumerate() {
            let mut flags = if key { FLAG_KEYFRAME } else { 0 };
            if index + 1 < count {
                flags |= FLAG_MORE;
            }
            let part = FrameRecord {
                frame_id: now,
                capture_ts_us: now,
                flags,
                payload: bytes.clone(),
            };
            out.extend(sink.record(&tiles, &part, now));
        }
        out
    };
    let out = feed(&mut sink, &pixels, true, 100);
    assert!(matches!(
        out.as_slice(),
        [SinkOutput::NewStream {
            gop: 3,
            codec: CodecKind::Tiles,
            ..
        }]
    ));
    // A second picture replaces the first one that the app has not pulled;
    // only the newest is kept and it keeps the group mark.
    pixels[..4].copy_from_slice(&[1, 2, 3, 255]);
    assert!(feed(&mut sink, &pixels, false, 200).is_empty());
    assert_eq!(queue.len(), 1);
    let (n, dst) = pull(&queue, 16 + size.bgra_len());
    assert_eq!(n as usize, 16 + size.bgra_len());
    let flags = word(&dst, 8);
    assert_eq!(flags & FLAG_TILES_BGRA, FLAG_TILES_BGRA);
    assert_eq!(flags & FLAG_NEW_STREAM, FLAG_NEW_STREAM);
    assert_eq!(dst[0..8], 200u64.to_le_bytes());
    // BGRA (1, 2, 3, 255) arrives as R, G, B, A.
    assert_eq!(dst[16..20], [3, 2, 1, 255]);
    // Records of an older group are stale.
    let stale = header(2, CodecKind::H264, size);
    assert!(sink
        .record(&stale, &record(9, 9, true, b"x"), 300)
        .is_empty());
    assert_eq!(queue.len(), 0);
    // A broken group asks once and skips until its next key frame.
    let broken = sink.broken(3, 400);
    assert!(matches!(broken, Some(SinkOutput::Keyframe(_))));
    assert!(sink.broken(3, 500).is_none(), "one request is open");
    assert_eq!(bgra_to_rgba(&[1, 2, 3, 4]), vec![3, 2, 1, 4]);
}
