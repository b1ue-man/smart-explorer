//! The LZ4 content block format (`lz4-v1`): round trip, the limits a
//! receiver enforces before and while it unpacks, the raw fall-back and what
//! the credit counts for a block.
use std::io::ErrorKind;
use std::time::{Duration, Instant};

use super::{random, text};
use crate::agent_proto::{
    credit_cost, pack_lz4, pack_raw, raw_len, unpack, BlockSender, Frame, MAX_RAW,
};
use crate::transfer::compress_policy::{Policy, METER_WINDOW};

#[test]
fn speed_task_lz4_block_roundtrip() {
    for raw in [text(MAX_RAW), text(1), text(777), random(MAX_RAW)] {
        let block = pack_lz4(&raw).map_or_else(|| pack_raw(&raw), |block| block);
        assert_eq!(unpack(&block).expect("unpack"), raw);
        assert_eq!(raw_len(&block).expect("raw length"), raw.len());
    }
    let packed = pack_lz4(&text(MAX_RAW)).expect("text shrinks");
    assert!(packed.len() < MAX_RAW / 10, "{}", packed.len());
    assert_eq!(packed[0], 1);
    assert_eq!(unpack(&pack_raw(b"")).expect("empty raw block"), b"");
}

#[test]
fn speed_task_lz4_rejects_raw_len_above_256_kib() {
    let mut block = pack_lz4(&text(MAX_RAW)).expect("packs");
    block[1..5].copy_from_slice(&(MAX_RAW as u32 + 1).to_le_bytes());
    assert_eq!(
        unpack(&block).expect_err("too long").kind(),
        ErrorKind::InvalidData
    );
    assert_eq!(
        raw_len(&block).expect_err("too long").kind(),
        ErrorKind::InvalidData
    );
    block[1..5].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        unpack(&block).expect_err("huge").kind(),
        ErrorKind::InvalidData
    );
    block[1..5].copy_from_slice(&0u32.to_le_bytes());
    assert!(unpack(&block).is_err(), "a zero length is no block");
    assert_eq!(
        unpack(&pack_raw(&vec![0; MAX_RAW + 1]))
            .expect_err("raw too long")
            .kind(),
        ErrorKind::InvalidData
    );
}

#[test]
fn speed_task_lz4_rejects_length_mismatch() {
    let raw = text(100_000);
    let block = pack_lz4(&raw).expect("packs");
    for announced in [raw.len() - 1, raw.len() + 1, raw.len() + 5000, 1] {
        let mut wrong = block.clone();
        wrong[1..5].copy_from_slice(&(announced as u32).to_le_bytes());
        assert_eq!(
            unpack(&wrong).expect_err("different length").kind(),
            ErrorKind::InvalidData,
            "announced {announced} for {}",
            raw.len()
        );
    }
}

#[test]
fn speed_task_lz4_rejects_truncated_blocks() {
    let block = pack_lz4(&text(MAX_RAW)).expect("packs");
    for cut in [0, 1, 3, 4, 5, 6, block.len() / 2, block.len() - 1] {
        assert_eq!(
            unpack(&block[..cut]).expect_err("cut off").kind(),
            ErrorKind::InvalidData,
            "cut at {cut} of {}",
            block.len()
        );
    }
    assert!(unpack(&[7, 1, 2, 3]).is_err(), "an unknown kind");
}

#[test]
fn speed_task_lz4_incompressible_block_is_sent_raw() {
    assert!(pack_lz4(&random(MAX_RAW)).is_none());

    // A sender whose policy decided to compress still sends data that does
    // not shrink as it is (a plain Data frame, no block).
    let mut policy = Policy::new();
    let start = Instant::now();
    policy.observe_compression(MAX_RAW, MAX_RAW * 3 / 10, Duration::from_micros(750));
    policy.record_send(1_000_000, Duration::from_millis(450), start);
    policy.record_send(11_500_000, Duration::from_millis(450), start + METER_WINDOW);
    let mut sender = BlockSender::with_policy(policy);
    sender.start_file();
    assert!(sender.try_pack(&text(MAX_RAW), 0).is_some(), "text shrinks");
    assert!(
        sender.try_pack(&random(MAX_RAW), MAX_RAW as u64).is_none(),
        "random data goes out raw"
    );
}

#[test]
fn speed_task_lz4_agent_credit_charges_the_raw_length() {
    let raw = text(MAX_RAW);
    let block = pack_lz4(&raw).expect("packs");
    let frame = Frame::DataLz4(block.clone());
    assert!(block.len() < 10_000);
    assert_eq!(
        credit_cost(&frame),
        MAX_RAW as u64,
        "not the few KiB on the wire"
    );
    let bytes = frame.encode(3).expect("encode");
    let mut framed = (bytes.len() as u32).to_le_bytes().to_vec();
    framed.extend_from_slice(&bytes);
    assert_eq!(
        crate::agent_proto::credit::encoded_cost(&framed),
        MAX_RAW as u64,
        "the writer of the sender charges the same"
    );
    assert!(credit_cost(&Frame::Data(raw)) > MAX_RAW as u64);
}
