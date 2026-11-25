use super::*;
use crate::screen::core::audio_wire::{AudioCodec, AudioPacket};

fn message(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index * 7 + 3) as u8).collect()
}

/// The data shards of every block in order, cut to the message length.
fn joined(packets: &[Vec<u8>]) -> Vec<u8> {
    let mut heads: Vec<(PacketHead, Vec<u8>)> = packets
        .iter()
        .map(|packet| {
            let (head, shard) = PacketHead::decode(packet).unwrap();
            (head, shard.to_vec())
        })
        .filter(|(head, _)| head.shard < head.data)
        .collect();
    heads.sort_by_key(|(head, _)| (head.block, head.shard));
    let len = heads[0].0.message_len as usize;
    let mut out: Vec<u8> = heads.into_iter().flat_map(|(_, shard)| shard).collect();
    out.truncate(len);
    out
}

#[test]
fn udp_task_packets_round_trip_and_tell_video_from_audio() {
    let shard = 64;
    // One byte, one shard, a partial last shard, several blocks.
    for len in [1, 64, 64 * 10 + 3, 64 * 150 + 17, 64 * 450] {
        let bytes = message(len);
        let packets = packetize(&bytes, shard, 42, 7, 0).unwrap();
        let layout = Layout::new(len, shard).unwrap();
        let expected: usize = (0..layout.blocks)
            .map(|block| {
                let (_, data) = layout.block(block);
                data + parity_for(data)
            })
            .sum();
        assert_eq!(packets.len(), expected, "{len}");
        // The blocks share the data evenly and cover it once.
        let mut next = 0;
        for block in 0..layout.blocks {
            let (first, count) = layout.block(block);
            assert_eq!(first, next);
            assert!(count <= MAX_BLOCK_DATA && count + 1 >= layout.data_shards / layout.blocks);
            next += count;
        }
        assert_eq!(next, layout.data_shards);
        for packet in &packets {
            let (head, payload) = PacketHead::decode(packet).unwrap();
            assert_eq!((head.frame_id, head.gop_id, head.generation), (42, 7, 0));
            assert_eq!(payload.len(), shard);
            assert!(layout.matches(&head), "{len} {head:?}");
            // A picture packet is no sound packet and back.
            assert!(is_video(packet));
            assert!(!AudioPacket::is_audio(packet));
            assert!(AudioPacket::decode(packet).is_err());
        }
        // Interleaved: the first packets are shard 0 of every block.
        for (block, packet) in packets.iter().take(layout.blocks).enumerate() {
            let (head, _) = PacketHead::decode(packet).unwrap();
            assert_eq!((head.block as usize, head.shard), (block, 0));
        }
        assert_eq!(joined(&packets), bytes, "{len}");
    }
    let sound = AudioPacket {
        codec: AudioCodec::Opus,
        sequence: 1,
        host_us: 2,
        payload: vec![0; 80],
    }
    .encode()
    .unwrap();
    assert!(!is_video(&sound));
    assert!(PacketHead::decode(&sound).is_err());
    // Parity: 20 %, at least one.
    assert_eq!([1, 5, 6, 10, 11, 100].map(parity_for), [1, 1, 2, 2, 3, 20]);
    // A packet with a short shard, another version or no bytes is refused.
    let mut packet = packetize(&message(100), shard, 1, 1, 0).unwrap().remove(0);
    let mut short = packet.clone();
    short.pop();
    assert!(PacketHead::decode(&short).is_err());
    packet[1] = 2;
    assert!(PacketHead::decode(&packet).is_err());
    assert!(packetize(&[], shard, 1, 1, 0).is_err());
    // Shard sizes of the carriers: the pair's 1166-byte inner packets; too
    // small packets carry nothing.
    assert_eq!(shard_len_for(1166), Some(1166 - PACKET_HEAD));
    assert_eq!(shard_len_for(PACKET_HEAD + MIN_SHARD - 1), None);
}
