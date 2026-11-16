//! Tests of the DMA-BUF capture (plan 2026-11-14, BK8b): offer bytes, the
//! choice DMA-BUF or shared memory, the fallback, the lifetime of held
//! buffers, "unchanged" from damage metadata and the import admission. They
//! need neither a compositor nor a GPU.
use super::*;
use crate::screen::core::capture_admission::{CaptureGeneration, SourceKey};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::os::pipewire_abi::SpaData;
use crate::screen::os::pipewire_stream::buffers::fixture;
use crate::screen::os::pipewire_stream::dmabuf_stream::planes;
use crate::screen::os::pipewire_stream::imports::fixture_support::Fixture;
use crate::screen::os::spa_pod::dmabuf::{
    damage_meta_pod, dmabuf_buffers_pod, enum_format_dmabuf_pod, parse_damage, parse_negotiated,
    Damage, DmaBufOffer, DATA_DMABUF, DRM_FORMAT_ARGB8888, DRM_FORMAT_XRGB8888,
};
use crate::screen::os::spa_pod::{enum_format_pod, parse_video_format, VideoFormat};
use std::sync::atomic::AtomicUsize;

/// The tests that switch the process-wide pipeline flag run one at a time.
pub(super) static PIPELINE: Mutex<()> = Mutex::new(());

const TILED: u64 = 0x0100_0000_0000_0002;

fn word(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn pad(out: &mut Vec<u8>) {
    while out.len() % 8 != 0 {
        out.push(0);
    }
}

fn id_pod(out: &mut Vec<u8>, id: u32) {
    word(out, 4);
    word(out, 3);
    word(out, id);
    word(out, 0);
}

fn object(body: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::new();
    word(&mut out, body.len() as u32);
    word(&mut out, 15);
    out.extend_from_slice(&body);
    pad(&mut out);
    out
}

fn target() -> ImportTarget {
    ImportTarget {
        node: Some(PathBuf::from("/dev/dri/renderD128")),
        formats: vec![
            (DRM_FORMAT_XRGB8888, vec![TILED, 0]),
            (DRM_FORMAT_ARGB8888, Vec::new()),
            (u32::from_le_bytes(*b"NV12"), vec![0]),
        ],
    }
}

fn contains_word(bytes: &[u8], value: u32) -> bool {
    bytes
        .chunks_exact(4)
        .any(|chunk| chunk == value.to_le_bytes())
}

#[test]
fn bildkette_task_dmabuf_offer_pod_lists_modifiers_mandatory_dont_fixate_then_shm() {
    // Written out from spa/pod/pod.h and spa/param/format.h (PipeWire 1.2.7).
    let mut body = Vec::new();
    word(&mut body, 0x40003); // SPA_TYPE_OBJECT_Format
    word(&mut body, 3); // SPA_PARAM_EnumFormat
    word(&mut body, 1);
    word(&mut body, 0);
    id_pod(&mut body, 2); // video
    word(&mut body, 2);
    word(&mut body, 0);
    id_pod(&mut body, 1); // raw
    word(&mut body, 0x20001);
    word(&mut body, 0);
    id_pod(&mut body, 8); // BGRx, a plain Id
    word(&mut body, 0x20002); // SPA_FORMAT_VIDEO_modifier
    word(&mut body, (1 << 3) | (1 << 4)); // MANDATORY | DONT_FIXATE
    word(&mut body, 16 + 3 * 8); // choice body: header 16, three Longs
    word(&mut body, 19); // Choice
    word(&mut body, 3); // Enum
    word(&mut body, 0);
    word(&mut body, 8);
    word(&mut body, 5); // Long
    for value in [TILED, TILED, 0] {
        body.extend_from_slice(&value.to_le_bytes());
    }
    word(&mut body, 0x20003);
    word(&mut body, 0);
    word(&mut body, 16 + 24);
    word(&mut body, 19);
    word(&mut body, 1);
    word(&mut body, 0);
    word(&mut body, 8);
    word(&mut body, 10);
    for value in [1920, 1080, 1, 1, 16384, 16384] {
        word(&mut body, value);
    }
    word(&mut body, 0x20004);
    word(&mut body, 0);
    word(&mut body, 16 + 24);
    word(&mut body, 19);
    word(&mut body, 1);
    word(&mut body, 0);
    word(&mut body, 8);
    word(&mut body, 11);
    for value in [0, 1, 0, 1, 1000, 1] {
        word(&mut body, value);
    }
    let offer = DmaBufOffer {
        format: VideoFormat::Bgrx,
        modifiers: vec![TILED, 0],
    };
    assert_eq!(enum_format_dmabuf_pod(&offer), Some(object(body)));
    // The offer of a session: DMA-BUF first, the shared memory one last.
    let pods = offer_pods(&offers_for(&target()));
    assert_eq!(pods.len(), 2);
    assert!(contains_word(&pods[0], 0x20002));
    assert_eq!(pods[1], enum_format_pod());
    assert!(!contains_word(&pods[1], 0x20002));
    // Nothing importable: no DMA-BUF pod at all.
    let none = DmaBufOffer {
        format: VideoFormat::Rgbx,
        modifiers: vec![0],
    };
    assert_eq!(enum_format_dmabuf_pod(&none), None);
    assert_eq!(offer_pods(&[]), vec![enum_format_pod()]);
}

fn format_pod(modifier: Option<(u32, u32, &[u64])>) -> Vec<u8> {
    let mut body = Vec::new();
    word(&mut body, 0x40003);
    word(&mut body, 4); // SPA_PARAM_Format
    word(&mut body, 0x20001);
    word(&mut body, 0);
    id_pod(&mut body, 8);
    if let Some((flags, kind, values)) = modifier {
        word(&mut body, 0x20002);
        word(&mut body, flags);
        if kind == u32::MAX {
            word(&mut body, 8);
            word(&mut body, 5);
        } else {
            word(&mut body, 16 + 8 * values.len() as u32);
            word(&mut body, 19);
            word(&mut body, kind);
            word(&mut body, 0);
            word(&mut body, 8);
            word(&mut body, 5);
        }
        for value in values {
            body.extend_from_slice(&value.to_le_bytes());
        }
    }
    word(&mut body, 0x20003);
    word(&mut body, 0);
    word(&mut body, 8);
    word(&mut body, 10);
    word(&mut body, 2560);
    word(&mut body, 1440);
    object(body)
}

#[test]
fn bildkette_task_dmabuf_negotiated_format_reads_modifier_and_fixation() {
    let size = FrameSize::new(2560, 1440);
    let plain = parse_negotiated(&format_pod(Some((8, u32::MAX, &[TILED])))).unwrap();
    assert_eq!(
        (
            plain.format,
            plain.size,
            plain.modifier,
            plain.fixation_pending
        ),
        (VideoFormat::Bgrx, size, Some(TILED), false)
    );
    let open = parse_negotiated(&format_pod(Some((24, 3, &[TILED, TILED, 0])))).unwrap();
    assert_eq!((open.modifier, open.fixation_pending), (Some(TILED), true));
    let fixated = parse_negotiated(&format_pod(Some((8, 0, &[0])))).unwrap();
    assert_eq!(
        (fixated.modifier, fixated.fixation_pending),
        (Some(0), false)
    );
    let shm = parse_negotiated(&format_pod(None)).unwrap();
    assert_eq!(shm.modifier, None);
    assert_eq!(
        parse_video_format(&format_pod(Some((8, u32::MAX, &[TILED])))),
        Some((VideoFormat::Bgrx, size))
    );
}

#[test]
fn bildkette_task_dmabuf_choice_dmabuf_only_for_a_gpu_import() {
    let offers = offers_for(&target());
    assert_eq!(
        offers,
        vec![DmaBufOffer {
            format: VideoFormat::Bgrx,
            modifiers: vec![TILED, 0]
        }]
    );
    assert_eq!(choose(true, &offers, true, false), BufferKind::DmaBuf);
    // Software coder, tiles or NVENC: a DMA-BUF would be read back.
    assert_eq!(choose(true, &offers, false, false), BufferKind::Shm);
    // The host mixes the pointer in: the CPU picture is needed.
    assert_eq!(choose(true, &offers, true, true), BufferKind::Shm);
    assert_eq!(choose(true, &[], true, false), BufferKind::Shm);
    assert_eq!(choose(false, &offers, true, false), BufferKind::Shm);
    let _serial = PIPELINE.lock().unwrap_or_else(PoisonError::into_inner);
    enable_gpu_pipeline(false);
    let session = DmaBufSession::default();
    let asked = AtomicBool::new(false);
    let pods = session.connect_pods(|| {
        asked.store(true, Ordering::SeqCst);
        target()
    });
    // Pipeline without GPU pictures: today's offer, the GPU is not probed.
    assert_eq!(pods, vec![enum_format_pod()]);
    assert!(!asked.load(Ordering::SeqCst));
    assert_eq!(
        VideoFormat::from_drm_fourcc(DRM_FORMAT_XRGB8888),
        Some(VideoFormat::Bgrx)
    );
    assert_eq!(VideoFormat::Bgra.drm_fourcc(), Some(DRM_FORMAT_ARGB8888));
    assert_eq!(VideoFormat::Rgbx.drm_fourcc(), None);
}

fn negotiated(modifier: Option<u64>) -> Negotiated {
    Negotiated {
        format: VideoFormat::Bgrx,
        size: FrameSize::new(1920, 1080),
        modifier,
        fixation_pending: false,
    }
}

#[test]
fn bildkette_task_dmabuf_fallback_renegotiates_shm_once_for_the_session() {
    let _serial = PIPELINE.lock().unwrap_or_else(PoisonError::into_inner);
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) {
            enable_gpu_pipeline(false);
        }
    }
    let _off = Off;
    enable_gpu_pipeline(true);
    let session = DmaBufSession::default();
    let pods = session.connect_pods(target);
    enable_gpu_pipeline(false);
    assert_eq!(pods.len(), 2);
    assert_eq!(session.node(), Some(PathBuf::from("/dev/dri/renderD128")));
    // The producer took the shared memory offer: SHM buffers, DMA-BUF stays.
    assert_eq!(session.on_format(&negotiated(None)), Ok(BufferKind::Shm));
    assert_eq!(session.dmabuf(), None);
    assert_eq!(
        session.on_format(&negotiated(Some(TILED))),
        Ok(BufferKind::DmaBuf)
    );
    assert_eq!(session.dmabuf(), Some((VideoFormat::Bgrx, TILED)));
    // A modifier this host did not offer is a fallback, not a stream error.
    assert_eq!(
        session.on_format(&negotiated(Some(0x0100_0000_0000_0009))),
        Err(Fallback::Modifier)
    );
    // Import fails: renegotiate with the shared memory offer, once.
    let shm = session.fall_back(Fallback::Import).unwrap();
    assert_eq!(shm, vec![enum_format_pod()]);
    assert_eq!(session.fallback(), Some(Fallback::Import));
    assert_eq!(session.fall_back(Fallback::Conversion), None);
    assert_eq!(session.dmabuf(), None);
    // For the rest of the session: no DMA-BUF format, no new DMA-BUF offer.
    assert_eq!(
        session.on_format(&negotiated(Some(TILED))),
        Err(Fallback::Modifier)
    );
    assert_eq!(session.on_format(&negotiated(None)), Ok(BufferKind::Shm));
    enable_gpu_pipeline(true);
    assert_eq!(session.connect_pods(target), vec![enum_format_pod()]);
    enable_gpu_pipeline(false);
    assert_eq!(Fallback::PointerBlend.text(), "dmabuf_pointer_blend");
}

#[derive(Default)]
struct Counter {
    given: Mutex<Vec<usize>>,
    calls: AtomicUsize,
}

impl BufferReturn for Counter {
    fn give_back(&self, buffer: usize) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.given.lock().unwrap().push(buffer);
    }
}

fn held(buffer: usize, counter: &Arc<Counter>) -> HeldBuffer {
    let fd = OwnedFd::from(std::fs::File::open("/dev/null").unwrap());
    let frame = DmaBufFrame {
        fd: -1,
        drm_format: DRM_FORMAT_XRGB8888,
        modifier: TILED,
        size: FrameSize::new(64, 32),
        planes: vec![(0, 256)],
        object_size: 8192,
    };
    let source = SourceKey::new("dmabuf-test", "portal:node").unwrap();
    let returner: Arc<dyn BufferReturn> = counter.clone();
    HeldBuffer::new(
        buffer,
        frame,
        fd,
        source,
        CaptureGeneration::new(3),
        returner,
    )
}

#[test]
fn bildkette_task_dmabuf_held_buffer_goes_back_once_after_the_gpu_read_it() {
    let counter = Arc::new(Counter::default());
    let first = held(0x10, &counter);
    // The frame names this program's own duplicate of the descriptor.
    assert!(first.frame().fd >= 0);
    let (source, generation) = first.identity();
    assert_eq!((source.source(), generation.value()), ("portal:node", 3));
    drop(first);
    assert_eq!(*counter.given.lock().unwrap(), vec![0x10]);
    // The loop thread queues itself: a disarmed buffer is not given back.
    assert_eq!(held(0x11, &counter).disarm(), 0x11);
    assert_eq!(counter.calls.load(Ordering::SeqCst), 1);
    let slot = GpuSlot::default();
    assert!(matches!(slot.take(true), GpuTaken::Unchanged));
    assert!(slot.publish(held(0x20, &counter)).is_none());
    // Only the newest is held: the older one comes back to be queued.
    let older = slot.publish(held(0x21, &counter)).unwrap();
    assert_eq!(older.disarm(), 0x20);
    let still = slot.still().unwrap();
    assert_eq!(still.0.planes, vec![(0, 256)]);
    let GpuTaken::Frame(taken) = slot.take(false) else {
        panic!("the newest picture goes to the encoder");
    };
    // The encoder drops it after the conversion: back exactly once.
    drop(taken);
    assert_eq!(counter.given.lock().unwrap().last(), Some(&0x21));
    assert_eq!(counter.calls.load(Ordering::SeqCst), 2);
    assert!(matches!(slot.take(false), GpuTaken::Unchanged));
    // A key frame without a new picture re-encodes the encoder's input.
    assert!(matches!(slot.take(true), GpuTaken::Repeat));
    // The still image still finds the last picture's descriptor.
    assert!(slot.still().is_some());
    assert!(slot.publish(held(0x22, &counter)).is_none());
    let cleared = slot.clear().unwrap();
    assert_eq!(cleared.disarm(), 0x22);
    assert!(slot.still().is_none());
    assert!(matches!(slot.take(true), GpuTaken::Unchanged));
    assert_eq!(counter.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn bildkette_task_dmabuf_damage_decides_unchanged_without_read_back() {
    let region = |x: i32, y: i32, width: u32, height: u32| {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes
    };
    let mut two = region(0, 0, 100, 50);
    two.extend(region(10, 10, 5, 5));
    two.extend(region(0, 0, 0, 0));
    two.extend(region(1, 1, 1, 1));
    assert_eq!(parse_damage(&two), Damage::Regions(2));
    assert_eq!(parse_damage(&region(3, 3, 0, 9)), Damage::Nothing);
    assert_eq!(parse_damage(&[0u8; 7]), Damage::Nothing);
    let session = DmaBufSession::default();
    // No damage metadata: every buffer is a new picture.
    assert!(session.changed(None));
    // A block the producer never filled reads as zero: still a picture.
    assert!(session.changed(Some(Damage::Nothing)));
    assert!(session.changed(Some(Damage::Regions(1))));
    // Once the producer showed it fills the block, no region = unchanged.
    assert!(!session.changed(Some(Damage::Nothing)));
    session.reset_damage();
    assert!(session.changed(Some(Damage::Nothing)));
    // The request for the block: ParamMeta, VideoDamage, 1 to 16 regions.
    let mut body = Vec::new();
    word(&mut body, 0x40005);
    word(&mut body, 6);
    word(&mut body, 1);
    word(&mut body, 0);
    id_pod(&mut body, 3);
    word(&mut body, 2);
    word(&mut body, 0);
    word(&mut body, 16 + 12);
    word(&mut body, 19);
    word(&mut body, 1);
    word(&mut body, 0);
    word(&mut body, 4);
    word(&mut body, 4);
    for value in [256, 16, 256] {
        word(&mut body, value);
    }
    pad(&mut body); // the choice pod is padded to 8 bytes
    assert_eq!(damage_meta_pod(), object(body));
    // Buffers for a DMA-BUF format: 4 (4 to 16) buffers (r1), data type
    // flags 1 << SPA_DATA_DmaBuf.
    let mut body = Vec::new();
    word(&mut body, 0x40004);
    word(&mut body, 5);
    word(&mut body, 1);
    word(&mut body, 0);
    word(&mut body, 16 + 12);
    word(&mut body, 19);
    word(&mut body, 1); // Range
    word(&mut body, 0);
    word(&mut body, 4);
    word(&mut body, 4);
    for value in [4, 4, 16] {
        word(&mut body, value);
    }
    pad(&mut body);
    word(&mut body, 6);
    word(&mut body, 0);
    word(&mut body, 16 + 4);
    word(&mut body, 19);
    word(&mut body, 4); // Flags
    word(&mut body, 0);
    word(&mut body, 4);
    word(&mut body, 4);
    word(&mut body, 1 << 3);
    pad(&mut body); // a pod is padded to 8 bytes inside its object
    assert_eq!(dmabuf_buffers_pod(), object(body));
}

#[test]
fn bildkette_task_dmabuf_planes_need_one_dmabuf_object() {
    let one = [(DATA_DMABUF, 7i64, 0u32, 7680i32)];
    assert_eq!(planes(&one), Ok(vec![(0, 7680)]));
    let two = [(DATA_DMABUF, 7, 0, 7680), (DATA_DMABUF, 7, 8_294_400, 3840)];
    assert_eq!(planes(&two), Ok(vec![(0, 7680), (8_294_400, 3840)]));
    for broken in [
        vec![(DATA_DMABUF, 7, 0, 7680), (DATA_DMABUF, 8, 0, 7680)],
        vec![(2, 7, 0, 7680)],
        vec![(DATA_DMABUF, 7, 0, 0)],
        vec![(DATA_DMABUF, 7, 0, 64); 5],
        Vec::new(),
    ] {
        assert_eq!(planes(&broken), Err(Fallback::Layout));
    }
}

#[test]
fn bildkette_task_dmabuf_import_is_admitted_and_counted_but_never_mapped() {
    let _serial = fixture::SERIAL
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let f = Fixture::new("pw-dmabuf");
    let fd = fixture::fd(8192);
    unsafe {
        fixture::add_kind(1, &fd, 1, DATA_DMABUF);
    }
    f.admit();
    // Counted as a native surface of the session like a mapping, not mapped.
    assert_eq!(fixture::observed().1, 0);
    assert!(f.native_live() >= 8192);
    use std::os::fd::AsRawFd;
    let plane = SpaData {
        type_: DATA_DMABUF,
        flags: 0,
        fd: i64::from(fd.as_raw_fd()),
        mapoffset: 0,
        maxsize: 0,
        data: std::ptr::null_mut(),
        chunk: std::ptr::null_mut(),
    };
    assert!(unsafe { f.imports.dmabuf_owner(&plane) }.is_ok());
    // The shared memory reader never reads through an unmapped DMA-BUF.
    let as_shm = SpaData {
        type_: 2,
        maxsize: 4096,
        ..plane
    };
    assert!(unsafe { f.imports.data_range(&as_shm) }.is_err());
    // A descriptor without an admitted import is refused.
    let foreign = fixture::fd(4096);
    let unknown = SpaData {
        fd: i64::from(foreign.as_raw_fd()),
        ..plane
    };
    assert!(unsafe { f.imports.dmabuf_owner(&unknown) }.is_err());
}
