//! Station times and chain facts of a session (plan 2026-11-14 „Bildkette“
//! BK0, 4.10a): what the host reports in `STATS` beside its counters, and the
//! viewer's request for the station times (`FRAME_TIMING`, `frame_timing_v1`).
//!
//! Announcement: the hello belongs to another package (BK6), so the host
//! announces `frame_timing_v1` in every `STATS` (`HostStats::frame_timing`
//! = `Some(1)`). A viewer sends `FRAME_TIMING` only after it read that
//! announcement; a host of v0.5.196 never announces it and never gets the
//! frame (it would refuse the unknown tag). A viewer of v0.5.196 ignores the
//! new `STATS` fields (serde ignores unknown fields).
//!
//! `FRAME_TIMING` (viewer to host, JSON): `{"on": true}` starts the station
//! times (and resets them), `{"on": false}` stops them; `{"on": true,
//! "probe_us": T}` is an input probe sent with the viewer clock `T` that
//! leaves the times running. The host answers the probe in the next `STATS`
//! (`input_raw`): host clock when the frame was handled minus `T`; the
//! viewer subtracts its clock offset (`PING`/`PONG`, host minus viewer).
//! Likewise `network_raw` is the viewer's arrival time of a frame's last
//! record minus the host's send time; the viewer adds the offset.
//!
//! `HostStations` carry p50/p95 in µs since the last `{"on": true}`; the
//! `ChainReport` names the stage of every station and what limits the rate.
use serde::{Deserialize, Serialize};

/// Viewer to host: the request for the station times (`frame_timing_v1`).
pub const FRAME_TIMING: u8 = 52;
/// Version a host announces in `HostStats::frame_timing`.
pub const FRAME_TIMING_VERSION: u8 = 1;

/// `FRAME_TIMING`: start/stop the station times, or one input probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameTimingRequest {
    pub on: bool,
    /// Viewer clock (µs) when the probe was sent; with it the frame is only
    /// a probe and does not reset the times.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_us: Option<u64>,
}

impl FrameTimingRequest {
    /// Starts (and resets) the station times.
    pub fn start() -> Self {
        Self {
            on: true,
            probe_us: None,
        }
    }

    /// One input probe sent at `sent_us` (viewer clock).
    pub fn probe(sent_us: u64) -> Self {
        Self {
            on: true,
            probe_us: Some(sent_us),
        }
    }
}

/// p50 and p95 of one station in µs (nearest rank).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationUs {
    pub p50_us: u64,
    pub p95_us: u64,
}

/// p50 and p95 of a difference of two clocks in µs; meaningful only after
/// the viewer's clock offset is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkewedUs {
    pub p50_us: i64,
    pub p95_us: i64,
}

impl SkewedUs {
    /// The station after adding `shift_us` (µs), never below zero.
    pub fn shifted(self, shift_us: i64) -> StationUs {
        let fix = |value: i64| u64::try_from(value.saturating_add(shift_us)).unwrap_or(0);
        StationUs {
            p50_us: fix(self.p50_us),
            p95_us: fix(self.p95_us),
        }
    }
}

/// The host's station times (`STATS`, only after `FRAME_TIMING` on).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostStations {
    /// The capture call of a picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<StationUs>,
    /// Colour conversion as a call of its own that ends synchronously
    /// (`convert_clock`: CPU conversion before OpenH264, SVT-AV1, the I420
    /// input of the Linux hardware encoders and Media Foundation's CPU input;
    /// the DMA-BUF import of the Linux VA encoders); absent where the
    /// conversion is part of the encoder or queued on the GPU.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub convert: Option<StationUs>,
    /// Scaling and encoding of a picture; the conversion too where
    /// `convert` is absent (`encode` + `convert` = the encoder call).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encode: Option<StationUs>,
    /// Host process wrote the encoded picture until the daemon read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_handoff: Option<StationUs>,
    /// Handing all records of a frame to the picture stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send: Option<StationUs>,
    /// Deviation of the capture times from the cadence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_jitter: Option<StationUs>,
    /// Injection of one `INPUT` batch on the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_inject: Option<StationUs>,
    /// Viewer arrival of the last record minus host send time (raw clocks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_raw: Option<SkewedUs>,
    /// Host handling of an input probe minus its viewer send time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_raw: Option<SkewedUs>,
}

impl HostStations {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// What the stations of a session use and what limits its rate (4.10a).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainReport {
    /// `CapturePath::key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_path: Option<String>,
    /// `CaptureFallback::key`: why the capture left its first path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_fallback: Option<String>,
    /// `gpu` or `cpu`: where the picture went to the encoder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub convert: Option<String>,
    /// The capture cannot limit its delivery to the cadence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_rate_capped: Option<bool>,
    /// The pointer is part of the picture (the portal offers no metadata).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_embedded: Option<bool>,
    /// GPU pictures read back to the CPU since the session started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readbacks: Option<u64>,
    /// Resets recovered by continuing the encoder's chain (B-36).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recoveries_continued: Option<u64>,
    /// Resets recovered with a key frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recoveries_keyframe: Option<u64>,
    /// The cadence of the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_fps: Option<u32>,
    /// The rate the viewer asked for (after the quality mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wanted_fps: Option<u32>,
    /// What the measured encoding time carries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encode_capacity_fps: Option<u32>,
    /// `viewer`, `source`, `encode_time` or `link`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps_limit: Option<String>,
    /// `ceiling` (the viewer's „Höchste Datenrate“), `codec_ceiling` (the
    /// codec pairing), `link` (rate control on the line); since plan
    /// 2026-11-17 also `ramp` (the start ramp), `content` (the picture does
    /// not fill the target), `encoder_ceiling` (the encoder's own ceiling)
    /// and `encoder_ignores` (the encoder exceeds its rate, se gives it less).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<String>,
    /// The viewer's „Höchste Datenrate“ in kbit/s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_ceiling_kbps: Option<u64>,
    /// The rate the encoder is given in kbit/s: the target times the
    /// correction of its honour meter (plan 2026-11-17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder_rate_kbps: Option<u64>,
    /// `pending`, `honoured`, `tolerated`, `corrected` or `ignores`
    /// (`rate_control::honour::HonourVerdict`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_honour: Option<String>,
    /// The host's display for this session (plan 2026-11-17 P5,
    /// `VirtualDisplayReport::chain_key`): `active:1920x1080:randr`, …;
    /// absent for a session whose viewer sent no wish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_display: Option<String>,
    /// The line's available rate the last bandwidth probe found, in kbit/s
    /// (`bandwidth_probe.rs`); absent before the first answer, for a viewer
    /// that reads no probe streams and from hosts before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured_kbps: Option<u64>,
    /// How long ago that answer arrived, in ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured_age_ms: Option<u64>,
    /// The „Übertragung“ the host runs (`delivery.rs`); absent from hosts
    /// before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<super::delivery::Delivery>,
    /// The carrier of the pictures under „UDP“ (`video_transport.rs`):
    /// `direct` (the session's own UDP pair), `quic_direct` (QUIC datagrams
    /// on a direct path), `relay` (QUIC datagrams through the relay, not
    /// UDP); absent while the pictures take the streams. The `udp_*`
    /// counters run from the session's start; absent before the viewer
    /// offered a carrier and from hosts before them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp_path: Option<String>,
    /// Pictures the viewer gave up incomplete, and those the host's
    /// carrier did not take.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp_lost_pictures: Option<u64>,
    /// Whole pictures the viewer skipped because their chain broke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp_skipped_pictures: Option<u64>,
    /// Pictures completed with packets rebuilt from parity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp_recovered_pictures: Option<u64>,
    /// Data packets that did not arrive, rebuilt or not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp_lost_packets: Option<u64>,
    /// Data packets rebuilt from parity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp_recovered_packets: Option<u64>,
    /// The carrier of the pictures under „TCP“: `direct` (the direct TCP
    /// link), `quic_stream` (QUIC picture streams on a direct path),
    /// `relay` (QUIC picture streams through the relay); absent under
    /// „UDP“ and from hosts before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcp_path: Option<String>,
    /// How the direct TCP link runs (`TcpPathKind`: `lan`, `ipv6`,
    /// `public`, `punched`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcp_link: Option<String>,
    /// Why the direct TCP link is missing, in German (`TcpLinkFailure`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcp_failure: Option<String>,
}

/// The capture API a session uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapturePath {
    DesktopDuplication,
    WindowsGraphicsCapture,
    PipewireDmabuf,
    PipewireShm,
    Xshm,
    MediaProjection,
    TestPicture,
    /// X11 without a usable MIT-SHM: plain `GetImage` (r1).
    X11GetImage,
}

impl CapturePath {
    /// The packing index is the position here: new paths go to the end.
    pub const ALL: [Self; 8] = [
        Self::DesktopDuplication,
        Self::WindowsGraphicsCapture,
        Self::PipewireDmabuf,
        Self::PipewireShm,
        Self::Xshm,
        Self::MediaProjection,
        Self::TestPicture,
        Self::X11GetImage,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::DesktopDuplication => "desktop_duplication",
            Self::WindowsGraphicsCapture => "windows_graphics_capture",
            Self::PipewireDmabuf => "pipewire_dmabuf",
            Self::PipewireShm => "pipewire_shm",
            Self::Xshm => "xshm",
            Self::MediaProjection => "media_projection",
            Self::TestPicture => "test_picture",
            Self::X11GetImage => "x11_get_image",
        }
    }
}

/// Why the capture left its first path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureFallback {
    DxgiUnsupported,
    DxgiAccessLost,
    DxgiAdapterChanged,
    DxgiFailed,
    /// Linux PipeWire left DMA-BUF for shared memory (r1; the causes of
    /// `os/linux_os/pipewire_dmabuf.rs`, `capture_route::note_fallback`).
    DmabufModifier,
    DmabufLayout,
    DmabufImport,
    DmabufConversion,
    DmabufConsumerCpu,
    DmabufPointerBlend,
    /// Plan 2026-11-17 P9: why a Windows picture went to the encoder as CPU
    /// pixels instead of staying on the GPU (`os/windows/capture_gpu_route.rs`
    /// `CpuReason`): no encoder of the host process takes textures, the
    /// encoder runs on another device, the GPU path failed earlier in the
    /// session, the capture device has no video support, the output is
    /// rotated, the capture is scaled, the pointer is blended on the CPU.
    GpuNotArmed,
    GpuOtherDevice,
    GpuDisabled,
    GpuNoVideo,
    GpuRotated,
    GpuScaled,
    GpuPointerBlend,
}

impl CaptureFallback {
    /// The packing index is the position here: new causes go to the end.
    pub const ALL: [Self; 17] = [
        Self::DxgiUnsupported,
        Self::DxgiAccessLost,
        Self::DxgiAdapterChanged,
        Self::DxgiFailed,
        Self::DmabufModifier,
        Self::DmabufLayout,
        Self::DmabufImport,
        Self::DmabufConversion,
        Self::DmabufConsumerCpu,
        Self::DmabufPointerBlend,
        Self::GpuNotArmed,
        Self::GpuOtherDevice,
        Self::GpuDisabled,
        Self::GpuNoVideo,
        Self::GpuRotated,
        Self::GpuScaled,
        Self::GpuPointerBlend,
    ];

    /// The cause of `key`; `None` for a text no cause has.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::DxgiUnsupported => "dxgi_unsupported",
            Self::DxgiAccessLost => "dxgi_access_lost",
            Self::DxgiAdapterChanged => "dxgi_adapter_changed",
            Self::DxgiFailed => "dxgi_failed",
            Self::DmabufModifier => "dmabuf_modifier",
            Self::DmabufLayout => "dmabuf_layout",
            Self::DmabufImport => "dmabuf_import",
            Self::DmabufConversion => "dmabuf_conversion",
            Self::DmabufConsumerCpu => "dmabuf_consumer_cpu",
            Self::DmabufPointerBlend => "dmabuf_pointer_blend",
            Self::GpuNotArmed => "gpu_not_armed",
            Self::GpuOtherDevice => "gpu_other_device",
            Self::GpuDisabled => "gpu_disabled",
            Self::GpuNoVideo => "gpu_no_video",
            Self::GpuRotated => "gpu_rotated",
            Self::GpuScaled => "gpu_scaled",
            Self::GpuPointerBlend => "gpu_pointer_blend",
        }
    }

    /// Whether the cause names the conversion of one picture (P9) rather
    /// than the capture's path: it holds only while pictures go as CPU
    /// pixels (`chain_meter::shown_fallback`).
    pub fn is_conversion(self) -> bool {
        matches!(
            self,
            Self::GpuNotArmed
                | Self::GpuOtherDevice
                | Self::GpuDisabled
                | Self::GpuNoVideo
                | Self::GpuRotated
                | Self::GpuScaled
                | Self::GpuPointerBlend
        )
    }
}

/// What the capture of the host process knows of itself; `None` is „not
/// reported“. Crosses the process boundary packed in one `u64`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureFacts {
    pub path: Option<CapturePath>,
    pub fallback: Option<CaptureFallback>,
    pub rate_capped: Option<bool>,
    pub cursor_embedded: Option<bool>,
    /// The last picture reached the encoder on the GPU.
    pub on_gpu: Option<bool>,
}

fn tri(value: Option<bool>) -> u64 {
    match value {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }
}

fn untri(bits: u64) -> Option<bool> {
    match bits & 3 {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

fn index_of<T: PartialEq + Copy>(all: &[T], value: Option<T>) -> u64 {
    value
        .and_then(|value| all.iter().position(|known| *known == value))
        .map_or(0, |index| index as u64 + 1)
}

fn from_index<T: Copy>(all: &[T], byte: u64) -> Option<T> {
    usize::try_from(byte & 0xff)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| all.get(index).copied())
}

impl CaptureFacts {
    /// Bits 0–7 path, 8–15 fallback, 16–17 rate cap, 18–19 embedded
    /// pointer, 20–21 GPU picture (0 unknown, 1 no, 2 yes).
    pub fn pack(&self) -> u64 {
        index_of(&CapturePath::ALL, self.path)
            | index_of(&CaptureFallback::ALL, self.fallback) << 8
            | tri(self.rate_capped) << 16
            | tri(self.cursor_embedded) << 18
            | tri(self.on_gpu) << 20
    }

    /// The facts of `pack`; unknown codes read as „not reported“.
    pub fn unpack(bits: u64) -> Self {
        Self {
            path: from_index(&CapturePath::ALL, bits),
            fallback: from_index(&CaptureFallback::ALL, bits >> 8),
            rate_capped: untri(bits >> 16),
            cursor_embedded: untri(bits >> 18),
            on_gpu: untri(bits >> 20),
        }
    }

    /// Keeps what `newer` does not know from `self`.
    pub fn merged(self, newer: Self) -> Self {
        Self {
            path: newer.path.or(self.path),
            fallback: newer.fallback.or(self.fallback),
            rate_capped: newer.rate_capped.or(self.rate_capped),
            cursor_embedded: newer.cursor_embedded.or(self.cursor_embedded),
            on_gpu: newer.on_gpu.or(self.on_gpu),
        }
    }
}
