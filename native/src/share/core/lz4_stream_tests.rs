//! LZ4 on Share content streams: when a stream is plain, what a packing
//! sender sends and what a receiver accepts.
use std::borrow::Cow;
use std::io::ErrorKind;
use std::time::{Duration, Instant};

use super::{classify, frame_for, negotiated, Content, Lz4Stream, TAG_DATA, TAG_DATA_LZ4};
use crate::share::fs::CHUNK;
use crate::share::wire::FsTransferCapabilities;
use crate::transfer::compress_policy::{Policy, METER_WINDOW};

fn text(len: usize) -> Vec<u8> {
    b"a Share carries files, and a file of text packs well. "
        .iter()
        .copied()
        .cycle()
        .take(len)
        .collect()
}

fn random(len: usize) -> Vec<u8> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

/// A stream whose wire limits (writer blocked), with a fast core and text.
fn wire_limited() -> Lz4Stream {
    let mut policy = Policy::new();
    let start = Instant::now();
    policy.observe_compression(CHUNK, CHUNK * 3 / 10, Duration::from_micros(750));
    policy.record_send(1_000_000, Duration::from_millis(450), start);
    policy.record_send(11_500_000, Duration::from_millis(450), start + METER_WINDOW);
    Lz4Stream::with_policy(policy)
}

#[test]
fn speed_task_lz4_share_stream_is_plain_without_the_capability() {
    let chunk = text(CHUNK);
    // No request flag, no stream: the plain frame with the very same bytes.
    assert!(!negotiated(false));
    let (tag, bytes) = frame_for(None, &chunk);
    assert_eq!(tag, TAG_DATA);
    assert!(
        matches!(bytes, Cow::Borrowed(_)),
        "no copy of a plain chunk"
    );
    assert_eq!(&*bytes, &chunk[..]);

    // A host before `lz4_v1` advertises nothing: serde reads it as off.
    let old: FsTransferCapabilities =
        serde_json::from_str(r#"{"v1":true,"admission":60,"batch_max_files":256}"#)
            .expect("an older host's capabilities");
    assert!(old.v1 && !old.lz4_v1 && !old.ranges_v1);
    let host = serde_json::to_string(&FsTransferCapabilities::host()).expect("host");
    assert!(host.contains("\"ranges_v1\":true"), "{host}");

    // A stream that did not negotiate refuses blocks.
    let refused = classify(TAG_DATA_LZ4, vec![0, 1, 2], false);
    assert_eq!(
        refused.err().map(|e| e.kind()),
        Some(ErrorKind::InvalidData)
    );
    assert!(matches!(
        classify(TAG_DATA, chunk.clone(), false),
        Ok(Content::Bytes(bytes)) if bytes == chunk
    ));
    assert!(matches!(
        classify(crate::share::framing::TAG_CTRL, vec![1], true),
        Ok(Content::Other(_))
    ));
}

#[test]
fn speed_task_lz4_share_stream_content_survives_compression() {
    let stream = wire_limited();
    let chunk = text(CHUNK);
    let (tag, packed) = frame_for(Some(&stream), &chunk);
    assert_eq!(tag, TAG_DATA_LZ4, "text on a wire-limited stream is packed");
    assert!(packed.len() < CHUNK / 5, "{}", packed.len());
    let Ok(Content::Bytes(unpacked)) = classify(tag, packed.into_owned(), true) else {
        panic!("a packed block must unpack");
    };
    assert_eq!(unpacked, chunk);

    // The next block keeps the decision; data that does not shrink goes plain.
    let noise = random(CHUNK);
    let (tag, bytes) = frame_for(Some(&stream), &noise);
    assert_eq!(tag, TAG_DATA);
    assert_eq!(&*bytes, &noise[..]);
}

#[test]
fn speed_task_lz4_share_blocks_are_checked_before_they_are_trusted() {
    let stream = wire_limited();
    let chunk = text(CHUNK);
    let (tag, packed) = frame_for(Some(&stream), &chunk);
    assert_eq!(tag, TAG_DATA_LZ4);
    let block = packed.into_owned();
    for broken in [
        Vec::new(),
        block[..3].to_vec(),
        block[..block.len() / 2].to_vec(),
        {
            let mut longer = block.clone();
            longer[1..5].copy_from_slice(&(CHUNK as u32 + 1).to_le_bytes());
            longer
        },
        {
            let mut shorter = block.clone();
            shorter[1..5].copy_from_slice(&(CHUNK as u32 - 1).to_le_bytes());
            shorter
        },
    ] {
        let verdict = classify(TAG_DATA_LZ4, broken, true);
        assert_eq!(
            verdict.err().map(|e| e.kind()),
            Some(ErrorKind::InvalidData)
        );
    }
}
