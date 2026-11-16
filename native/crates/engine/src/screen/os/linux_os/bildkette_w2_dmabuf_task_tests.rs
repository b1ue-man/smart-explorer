//! Task suite „Bildkette“ merge W2 (Linux, Wayland): while DMA-BUF is
//! negotiated the stream hands out GPU pictures only (never a CPU picture),
//! a consumer of the picture ends DMA-BUF for the session through the
//! stream (renegotiation to shared memory, no portal dialog), and the route
//! with its cause lies ready for the statistics (`core::capture_route`).
//! No compositor, no GPU. Not registered yet (`docs/tests/register.tsv`):
//! the manager adds the rows.
use std::path::PathBuf;
use std::sync::{Arc, PoisonError};

use super::super::dmabuf_stream::{GpuCapture, StreamFallback};
use super::super::LatestFrame;
use super::*;
use crate::screen::core::capture_route::{self, CaptureRoute};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::os::linux_hw_codecs::{DmaBufCause, DmaBufStream};
use crate::screen::os::spa_pod::dmabuf::DRM_FORMAT_XRGB8888;

const TILED: u64 = 0x0100_0000_0000_0002;

fn target() -> ImportTarget {
    ImportTarget {
        node: Some(PathBuf::from("/dev/dri/renderD128")),
        formats: vec![(DRM_FORMAT_XRGB8888, vec![TILED])],
    }
}

#[test]
fn bildkette_task_linux_w2_dmabuf_stream_falls_back_to_shared_memory_for_the_session() {
    let _serial = super::tests::PIPELINE
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            enable_gpu_pipeline(false);
            capture_route::reset_for_test();
        }
    }
    let _reset = Reset;
    capture_route::reset_for_test();
    enable_gpu_pipeline(true);
    let latest = Arc::new(LatestFrame::default());
    let pods = latest.dmabuf.connect_pods(target);
    enable_gpu_pipeline(false);
    assert_eq!(pods.len(), 2, "DMA-BUF offer before the shared memory one");
    let negotiated = Negotiated {
        format: VideoFormat::Bgrx,
        size: FrameSize::new(1920, 1080),
        modifier: Some(TILED),
        fixation_pending: false,
    };
    assert_eq!(latest.dmabuf.on_format(&negotiated), Ok(BufferKind::DmaBuf));
    capture_route::note_route(CaptureRoute::DmaBuf);
    let grant = crate::screen::core::capture_test_support::grant(FrameSize::new(1920, 1080), None);
    // DMA-BUF negotiated, no picture yet: the GPU route answers, no CPU
    // picture is taken from the shared memory slot.
    assert!(matches!(
        latest.take_gpu_picture(false, &grant),
        Ok(Some(GpuCapture::Unchanged))
    ));
    // A CPU consumer (or a failed import) ends DMA-BUF through the stream.
    StreamFallback(Arc::downgrade(&latest)).fall_back(DmaBufCause::Consumer);
    assert_eq!(latest.dmabuf.fallback(), Some(Fallback::Consumer));
    assert_eq!(latest.dmabuf.dmabuf(), None);
    // From now on the shared memory route of the session.
    assert!(matches!(latest.take_gpu_picture(true, &grant), Ok(None)));
    assert_eq!(
        latest.dmabuf.on_format(&negotiated),
        Err(Fallback::Modifier)
    );
    // A second cause changes nothing; the first one is reported.
    StreamFallback(Arc::downgrade(&latest)).fall_back(DmaBufCause::Import);
    assert_eq!(latest.dmabuf.fallback(), Some(Fallback::Consumer));
    let report = capture_route::report();
    assert_eq!(report.route, Some(CaptureRoute::SharedMemory));
    assert_eq!(report.fallback, Some("dmabuf_consumer_cpu"));
    // The encoder's causes map onto the stream's.
    assert_eq!(Fallback::from(DmaBufCause::Import), Fallback::Import);
    assert_eq!(
        Fallback::from(DmaBufCause::Conversion),
        Fallback::Conversion
    );
    assert_eq!(Fallback::from(DmaBufCause::Consumer), Fallback::Consumer);
    // A picture outliving its stream changes nothing.
    let gone = Arc::downgrade(&latest);
    drop(latest);
    StreamFallback(gone).fall_back(DmaBufCause::Import);
    // The keys of the report.
    assert_eq!(CaptureRoute::DmaBuf.key(), "dmabuf");
    assert_eq!(CaptureRoute::SharedMemory.key(), "shm");
    capture_route::note_cursor_embedded(true);
    assert_eq!(capture_route::report().cursor_embedded, Some(true));
    capture_route::note_cursor_embedded(false);
    assert_eq!(capture_route::report().cursor_embedded, Some(false));
}
