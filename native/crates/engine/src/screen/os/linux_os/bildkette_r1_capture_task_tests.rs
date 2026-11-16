//! Task suite „Bildkette“ r1, Linux host without compositor or X server:
//! the capture path the capture notes (`core::capture_route`: PipeWire
//! DMA-BUF or SHM with the fallback cause, X11 MIT-SHM or `GetImage`) and
//! whether the pointer is part of the picture become the `CaptureFacts` of
//! the host process, cross the process boundary packed and reach
//! `ChainReport` (`HostStats::chain`, `se screen sessions --json`) like the
//! Windows facts; the DMA-BUF `Buffers` answer asks the compositor for as
//! many buffers as the path holds plus one. Not registered yet
//! (`docs/tests/register.tsv`): the manager adds the rows.
use super::facts_of;
use crate::screen::core::capture_route::{CaptureRoute, CaptureRouteReport};
use crate::screen::core::chain_meter::{ChainMeter, RateFacts};
use crate::screen::core::frame_timing::{CaptureFacts, CaptureFallback, CapturePath};
use crate::screen::os::spa_pod::dmabuf::{
    dmabuf_buffers_pod, DMABUF_BUFFERS_MAX, DMABUF_BUFFERS_MIN, DMABUF_HELD,
};

fn report(route: CaptureRoute, fallback: Option<&'static str>) -> CaptureRouteReport {
    CaptureRouteReport {
        route: Some(route),
        fallback,
        cursor_embedded: Some(true),
    }
}

/// The chain report of a session whose host process packed `facts`.
fn chain_of(facts: CaptureFacts) -> crate::screen::core::frame_timing::ChainReport {
    let mut meter = ChainMeter::default();
    meter.set_facts(CaptureFacts::unpack(facts.pack()));
    meter.report(&RateFacts::default())
}

#[test]
fn bildkette_task_r1_linux_capture_path_reaches_the_chain() {
    let cases = [
        (CaptureRoute::DmaBuf, None, CapturePath::PipewireDmabuf),
        (
            CaptureRoute::SharedMemory,
            Some("dmabuf_import"),
            CapturePath::PipewireShm,
        ),
        (CaptureRoute::X11Shm, None, CapturePath::Xshm),
        (CaptureRoute::X11GetImage, None, CapturePath::X11GetImage),
    ];
    for (route, fallback, path) in cases {
        let facts = facts_of(&report(route, fallback));
        assert_eq!(facts.path, Some(path), "{route:?}");
        assert_eq!(facts.cursor_embedded, Some(true));
        assert_eq!(facts.rate_capped, None, "PipeWire and X11 pace themselves");
        let chain = chain_of(facts);
        assert_eq!(chain.capture_path.as_deref(), Some(path.key()));
        assert_eq!(chain.capture_fallback.as_deref(), fallback);
        assert_eq!(chain.cursor_embedded, Some(true));
    }
    assert_eq!(CapturePath::X11GetImage.key(), "x11_get_image");
    assert_eq!(CaptureRoute::X11Shm.key(), "xshm");
    // Every cause of `pipewire_dmabuf.rs` has its fallback in the chain.
    for (text, cause) in [
        ("dmabuf_modifier", CaptureFallback::DmabufModifier),
        ("dmabuf_layout", CaptureFallback::DmabufLayout),
        ("dmabuf_import", CaptureFallback::DmabufImport),
        ("dmabuf_conversion", CaptureFallback::DmabufConversion),
        ("dmabuf_consumer_cpu", CaptureFallback::DmabufConsumerCpu),
        ("dmabuf_pointer_blend", CaptureFallback::DmabufPointerBlend),
    ] {
        let facts = facts_of(&report(CaptureRoute::SharedMemory, Some(text)));
        assert_eq!(facts.fallback, Some(cause), "{text}");
        assert_eq!(CaptureFacts::unpack(facts.pack()).fallback, Some(cause));
    }
    // Nothing noted (a process without capture, before the first format):
    // „not reported“, never a guessed path.
    let empty = facts_of(&CaptureRouteReport::default());
    assert_eq!(empty, CaptureFacts::default());
    assert_eq!(chain_of(empty).capture_path, None);
    assert_eq!(chain_of(empty).cursor_embedded, None);
    // An unknown cause text is no cause.
    let odd = facts_of(&report(CaptureRoute::SharedMemory, Some("dmabuf_new")));
    assert_eq!(odd.fallback, None);
}

/// The `u32` words of a pod.
fn words(pod: &[u8]) -> Vec<u32> {
    pod.chunks_exact(4)
        .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
        .collect()
}

#[test]
fn bildkette_task_r1_linux_dmabuf_offer_asks_for_enough_buffers() {
    // Newest picture in the slot, waiting picture, encoder import, and one
    // the compositor renders into.
    assert_eq!(DMABUF_HELD, 3);
    assert_eq!(DMABUF_BUFFERS_MIN, DMABUF_HELD + 1);
    assert!(DMABUF_BUFFERS_MIN <= DMABUF_BUFFERS_MAX);
    // Inside the range mutter offers (2 to 16), so the formats intersect.
    assert!((2..=16).contains(&DMABUF_BUFFERS_MIN) && DMABUF_BUFFERS_MAX >= 16);
    let words = words(&dmabuf_buffers_pod());
    // SPA_PARAM_BUFFERS_buffers (1): a range choice of Int, wished and
    // least 4, most 16, before the data type (6).
    let buffers = [1, 0, 16 + 12, 19, 1, 0, 4, 4, 4, 4, 16];
    let at = words
        .windows(buffers.len())
        .position(|window| window == buffers)
        .expect("the pod names the number of buffers");
    let data_type = words
        .windows(2)
        .position(|window| window == [6, 0])
        .expect("the pod names the data type");
    assert!(at < data_type);
}
