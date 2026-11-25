use super::{carrier_choice, CarrierChoice};
use crate::screen::core::delivery::Delivery;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::wire_binary::StreamKind;

/// User 2026-11-25 („TCP und UDP, DIREKT“, „NUTZ das“): under „TCP“ every
/// `video` picture takes the direct TCP link: H.264, HEVC, AV1, the tiles
/// („Verlustfrei“) and pictures with hybrid regions. Under „UDP“ only the
/// video codecs without hybrid regions go as datagrams, the tiles and the
/// hybrid pictures keep the picture streams as before. Previews and stills
/// keep the streams under both.
#[test]
fn video_tcp_task_carrier_choice_per_codec_and_delivery() {
    use CarrierChoice::{Datagrams, Records, Streams};
    // Codec, hybrid regions, under „UDP“, under „TCP“.
    let rows = [
        (CodecKind::H264, false, Datagrams, Records),
        (CodecKind::Hevc, false, Datagrams, Records),
        (CodecKind::Av1, false, Datagrams, Records),
        (CodecKind::Tiles, false, Streams, Records),
        (CodecKind::H264, true, Streams, Records),
        (CodecKind::Hevc, true, Streams, Records),
        (CodecKind::Av1, true, Streams, Records),
        (CodecKind::Tiles, true, Streams, Records),
    ];
    for (codec, hybrid, realtime, reliable) in rows {
        assert_eq!(
            carrier_choice(StreamKind::Video, codec, hybrid, Delivery::Realtime),
            realtime,
            "UDP, {codec:?}, hybrid {hybrid}"
        );
        assert_eq!(
            carrier_choice(StreamKind::Video, codec, hybrid, Delivery::Reliable),
            reliable,
            "TCP, {codec:?}, hybrid {hybrid}"
        );
        for kind in [StreamKind::Preview, StreamKind::Snapshot] {
            for delivery in [Delivery::Realtime, Delivery::Reliable] {
                assert_eq!(
                    carrier_choice(kind, codec, hybrid, delivery),
                    Streams,
                    "{kind:?}, {delivery:?}, {codec:?}, hybrid {hybrid}"
                );
            }
        }
    }
}
