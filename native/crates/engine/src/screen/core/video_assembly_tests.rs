use super::*;
use crate::screen::core::video_transport::video_packet::packetize;

const SHARD: usize = 64;
const MS: u64 = 1_000;

fn message(frame: u64, len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| (index as u64 * 13 + frame) as u8)
        .collect()
}

fn packets(frame: u64, gop: u64, len: usize) -> Vec<Vec<u8>> {
    packetize(&message(frame, len), SHARD, frame, gop, 0).unwrap()
}

fn complete(out: &[Assembled]) -> Vec<(u64, usize)> {
    out.iter()
        .filter_map(|assembled| match assembled {
            Assembled::Complete {
                frame_id,
                recovered,
                ..
            } => Some((*frame_id, *recovered)),
            Assembled::GaveUp { .. } => None,
        })
        .collect()
}

fn given_up(out: &[Assembled]) -> Vec<u64> {
    out.iter()
        .filter_map(|assembled| match assembled {
            Assembled::GaveUp { frame_id, .. } => Some(*frame_id),
            Assembled::Complete { .. } => None,
        })
        .collect()
}

#[test]
fn udp_task_assembly_completes_in_order_and_rebuilds_lost_packets() {
    let mut assembler = Assembler::new(1 << 20);
    // 10 data shards and 2 parity shards; two data packets lost.
    let mut first = packets(1, 1, SHARD * 10);
    assert_eq!(first.len(), 12);
    first.remove(3);
    first.remove(0);
    let mut out = Vec::new();
    // Out of order: the last packets first.
    for packet in first.iter().rev() {
        out.extend(assembler.push(packet, 0));
    }
    assert_eq!(complete(&out), vec![(1, 2)]);
    match &out[0] {
        Assembled::Complete { message: got, .. } => assert_eq!(got, &message(1, SHARD * 10)),
        other => panic!("{other:?}"),
    }
    assert_eq!(assembler.counters.recovered_packets, 2);
    assert_eq!(assembler.counters.recovered_pictures, 1);
    // A late packet of a finished picture is counted, not assembled.
    assert!(assembler.push(&first[0], MS).is_empty());
    assert_eq!(assembler.counters.late_packets, 1);
    // Picture 2 lacks three of its 12 packets; picture 3 completes first:
    // 2 is given up before 3 goes on, and its late packets stay late.
    let second = packets(2, 1, SHARD * 10);
    for packet in &second[3..] {
        assert!(assembler.push(packet, 2 * MS).is_empty());
    }
    let mut out = Vec::new();
    for packet in packets(3, 1, SHARD * 2) {
        out.extend(assembler.push(&packet, 3 * MS));
    }
    assert_eq!(given_up(&out), vec![2]);
    assert_eq!(complete(&out), vec![(3, 0)]);
    assert!(matches!(
        out[0],
        Assembled::GaveUp {
            frame_id: 2,
            missing: 3,
            ..
        }
    ));
    assert!(assembler.push(&second[0], 4 * MS).is_empty());
    assert_eq!(assembler.counters.lost_pictures, 1);
    assert_eq!(assembler.assembling(), 0);
    // A one-shard picture survives the loss of its only data packet.
    let small = packets(4, 2, 10);
    assert_eq!(small.len(), 2);
    let out = assembler.push(&small[1], 5 * MS);
    assert_eq!(complete(&out), vec![(4, 1)]);
}

#[test]
fn udp_task_assembly_gives_up_after_silence() {
    let mut assembler = Assembler::new(1 << 20);
    assert_eq!(assembler.give_up_us(), MIN_GIVE_UP_US);
    let first = packets(1, 1, SHARD * 10);
    let second = packets(2, 1, SHARD * 10);
    assembler.push(&first[0], 0);
    assembler.push(&second[0], 10 * MS);
    assert_eq!(assembler.deadline_us(), Some(MIN_GIVE_UP_US));
    assert!(assembler.expire(49 * MS).is_empty());
    // Picture 1 is silent for 50 ms: it alone goes, 2 still waits.
    assert_eq!(given_up(&assembler.expire(50 * MS)), vec![1]);
    assert_eq!(assembler.assembling(), 1);
    // Picture 2 goes once silent; a newer silent picture takes the older
    // ones along.
    assembler.push(&packets(3, 1, SHARD * 10)[0], 30 * MS);
    assert_eq!(given_up(&assembler.expire(90 * MS)), vec![2, 3]);
    // 2026-11-25: time the task waited for the decoder is no silence: the
    // packets of that time were still unread in the socket buffer.
    let mut waiting = Assembler::new(1 << 20);
    waiting.push(&packets(5, 1, SHARD * 10)[0], 0);
    waiting.postpone(200 * MS);
    assert!(waiting.expire(240 * MS).is_empty());
    assert_eq!(given_up(&waiting.expire(250 * MS)), vec![5]);
    // Pictures 100 ms apart: the wait grows to two of their intervals.
    let mut assembler = Assembler::new(1 << 20);
    for frame in 1..40u64 {
        for packet in packets(frame, 1, SHARD) {
            assembler.push(&packet, frame * 100 * MS);
        }
    }
    assert!(
        assembler.give_up_us() > 190 * MS,
        "{}",
        assembler.give_up_us()
    );
    assert!(assembler.give_up_us() <= 200 * MS);
}

#[test]
fn udp_task_assembly_memory_is_bounded() {
    let mut assembler = Assembler::new(SHARD * 20);
    let mut gave_up = Vec::new();
    for frame in 1..=20u64 {
        let out = assembler.push(&packets(frame, 1, SHARD * 10)[0], frame * MS);
        gave_up.extend(given_up(&out));
        assert!(assembler.assembling() <= MAX_ASSEMBLING);
    }
    // The oldest went first, each once.
    assert_eq!(gave_up, (1..=12).collect::<Vec<u64>>());
    // A picture larger than the viewer accepts is refused packet by packet.
    let refused = assembler.counters.refused_packets;
    assert!(assembler
        .push(&packets(30, 1, SHARD * 21)[0], 30 * MS)
        .is_empty());
    assert_eq!(assembler.counters.refused_packets, refused + 1);
    // A newer cut of a picture replaces the older one.
    let mut assembler = Assembler::new(1 << 20);
    let old = packetize(&message(5, SHARD * 4), SHARD, 5, 1, 0).unwrap();
    let new = packetize(&message(5, SHARD * 4), SHARD / 2, 5, 1, 1).unwrap();
    assembler.push(&old[0], 0);
    let mut out = Vec::new();
    for packet in &new {
        out.extend(assembler.push(packet, MS));
    }
    assert_eq!(complete(&out), vec![(5, 0)]);
    // A packet of an older cut than one in assembly is refused.
    let mut assembler = Assembler::new(1 << 20);
    assembler.push(&new[0], 0);
    let refused = assembler.counters.refused_packets;
    assembler.push(&old[1], MS);
    assert_eq!(assembler.counters.refused_packets, refused + 1);
}

#[test]
fn udp_task_assembly_gives_the_tail_picture_up() {
    let mut assembler = Assembler::new(1 << 20);
    for packet in packets(1, 1, SHARD) {
        assembler.push(&packet, 0);
    }
    assert!(assembler.finished(1));
    // Of picture 2, the host's last before a still screen, one packet came.
    let second = packets(2, 1, SHARD * 10);
    assembler.push(&second[0], MS);
    assert!(!assembler.finished(2));
    let (out, present) = assembler.abandon_through(2);
    assert!(present);
    assert_eq!(given_up(&out), vec![2]);
    assert!(assembler.finished(2));
    assert!(assembler.push(&second[1], 2 * MS).is_empty());
    // Not one packet of picture 3 came: given up all the same, counted lost.
    let lost = assembler.counters.lost_pictures;
    let (out, present) = assembler.abandon_through(3);
    assert!(!present);
    assert!(out.is_empty());
    assert_eq!(assembler.counters.lost_pictures, lost + 1);
    assert!(assembler.finished(3));
    // Its packets that come after all are late.
    let late = assembler.counters.late_packets;
    assembler.push(&packets(3, 1, SHARD)[0], 3 * MS);
    assert_eq!(assembler.counters.late_packets, late + 1);
}
