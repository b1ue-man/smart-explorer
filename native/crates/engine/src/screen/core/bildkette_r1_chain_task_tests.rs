//! Task suite „Bildkette“ r1, platform-neutral: the conversion time of an
//! encoding is measured apart from it where the conversion is a call of its
//! own (`convert_clock`; station `convert`, `encode` without it), crosses
//! the process boundary in the stream mark, and the new capture paths and
//! fallback causes of Linux and Android pack and read back. Not registered
//! yet (`docs/tests/register.tsv`): the manager adds the rows.
use super::super::chain_meter::{ChainMeter, Station};
use super::super::frame_budget::FrameLimits;
use super::super::frame_timing::{CaptureFacts, CaptureFallback, CapturePath};
use super::super::pixel_convert::{bgra_to_i420_parallel, BgraImage, FrameSize, YuvMatrix};
use super::super::worker_wire::StreamMark;
use super::{add, arm, encode_without, take, Timed};

#[test]
fn bildkette_task_r1_convert_station_apart_from_encode() {
    // Not armed: a conversion outside an encoding counts nowhere.
    let image = BgraImage::new(FrameSize::new(64, 32)).unwrap();
    bgra_to_i420_parallel(&image.view(), YuvMatrix::Bt709, FrameLimits::LEGACY).unwrap();
    assert_eq!(take(), None);
    // Armed without a conversion of its own (encoder-internal): absent.
    arm();
    assert_eq!(take(), None);
    // Armed: the CPU conversion before a software encoder measures itself.
    arm();
    bgra_to_i420_parallel(&image.view(), YuvMatrix::Bt709, FrameLimits::LEGACY).unwrap();
    assert!(take().is_some());
    assert_eq!(take(), None, "take disarms");
    // Several conversions of one encoding add up.
    arm();
    add(300);
    drop(Timed::start());
    add(200);
    assert!(take().is_some_and(|micros| micros >= 500));
    // `encode` is the encoder call without the conversion.
    assert_eq!(encode_without(9_000, Some(2_000)), 7_000);
    assert_eq!(encode_without(9_000, None), 9_000);
    assert_eq!(encode_without(1_000, Some(2_000)), 0);
    // Another thread's clock is its own.
    arm();
    std::thread::spawn(|| add(1_000)).join().unwrap();
    assert_eq!(take(), None);

    // The station reaches the meter and `HostStations::convert`.
    let mut meter = ChainMeter::default();
    meter.request(true);
    meter.record_us(Station::Convert, 1_500);
    meter.record_us(Station::Encode, 4_000);
    let stations = meter.stations().unwrap();
    assert_eq!(stations.convert.unwrap().p50_us, 1_500);
    assert_eq!(stations.encode.unwrap().p50_us, 4_000);
    meter.request(true);
    assert!(meter.stations().is_none(), "on resets the conversion too");

    // The host process sends the conversion within `encode_us`.
    let mark = StreamMark {
        frame_id: 3,
        encode_us: 9_000,
        convert_us: 2_000,
        ..StreamMark::default()
    };
    assert_eq!(StreamMark::BYTES, 72);
    assert_eq!(StreamMark::decode(&mark.encode()).unwrap(), mark);
    assert!(StreamMark::decode(&mark.encode()[..64]).is_err());
}

#[test]
fn bildkette_task_r1_new_paths_and_causes_pack() {
    for path in [
        CapturePath::PipewireDmabuf,
        CapturePath::PipewireShm,
        CapturePath::Xshm,
        CapturePath::X11GetImage,
        CapturePath::MediaProjection,
    ] {
        let facts = CaptureFacts {
            path: Some(path),
            cursor_embedded: Some(false),
            ..CaptureFacts::default()
        };
        assert_eq!(CaptureFacts::unpack(facts.pack()), facts, "{path:?}");
    }
    assert_eq!(CapturePath::MediaProjection.key(), "media_projection");
    for cause in CaptureFallback::ALL {
        assert_eq!(CaptureFallback::from_key(cause.key()), Some(cause));
    }
    assert_eq!(CaptureFallback::from_key("dxgi"), None);
    // The packing indices of the paths and causes of BK0 stay.
    assert_eq!(CapturePath::ALL[0], CapturePath::DesktopDuplication);
    assert_eq!(CapturePath::ALL[6], CapturePath::TestPicture);
    assert_eq!(CaptureFallback::ALL[3], CaptureFallback::DxgiFailed);
}
