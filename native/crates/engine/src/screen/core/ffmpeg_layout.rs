//! Layout facts of the pinned FFmpeg build that the Windows loader checks
//! before its first call (plan 2026-11-17 P8; sources in the head of
//! `ffmpeg_plan.rs`): the byte offsets of the fields it reads or writes and
//! the fingerprints of a fresh frame and packet.

/// Byte offsets the loader reads, writes or checks (module head).
pub mod layout {
    pub const FRAME_DATA: usize = 0;
    pub const FRAME_LINESIZE: usize = 64;
    pub const FRAME_EXTENDED_DATA: usize = 96;
    pub const FRAME_WIDTH: usize = 104;
    pub const FRAME_HEIGHT: usize = 108;
    pub const FRAME_FORMAT: usize = 116;
    pub const FRAME_SAMPLE_ASPECT: usize = 124;
    pub const FRAME_PTS: usize = 136;
    pub const FRAME_PKT_DTS: usize = 144;
    pub const FRAME_TIME_BASE: usize = 152;
    pub const FRAME_COLOR_RANGE: usize = 280;
    pub const FRAME_COLOR_PRIMARIES: usize = 284;
    pub const FRAME_COLOR_TRC: usize = 288;
    pub const FRAME_COLORSPACE: usize = 292;
    pub const FRAME_BEST_EFFORT: usize = 304;
    /// Bytes of a frame the check reads.
    pub const FRAME_PROBE: usize = 312;
    pub const PACKET_BUF: usize = 0;
    pub const PACKET_PTS: usize = 8;
    pub const PACKET_DTS: usize = 16;
    pub const PACKET_DATA: usize = 24;
    pub const PACKET_SIZE: usize = 32;
    pub const PACKET_FLAGS: usize = 40;
    pub const PACKET_POS: usize = 72;
    pub const PACKET_TIME_BASE: usize = 96;
    /// `sizeof(AVPacket)`: bytes of a packet the check reads.
    pub const PACKET_PROBE: usize = 104;
    pub const BUFFER_DATA: usize = 8;
    pub const HWDEVICE_HWCTX: usize = 16;
    pub const D3D11VA_DEVICE: usize = 0;
    pub const D3D11VA_DEVICE_CONTEXT: usize = 8;
    pub const D3D11VA_LOCK: usize = 32;
    pub const D3D11VA_UNLOCK: usize = 40;
    pub const D3D11VA_LOCK_CTX: usize = 48;
    pub const OPTION_OFFSET: usize = 16;
}

/// `AV_NOPTS_VALUE`.
pub const NOPTS: i64 = i64::MIN;

fn bytes<const N: usize>(probe: &[u8], at: usize) -> Option<[u8; N]> {
    probe.get(at..at + N)?.try_into().ok()
}

pub fn read_i32(probe: &[u8], at: usize) -> Option<i32> {
    bytes::<4>(probe, at).map(i32::from_le_bytes)
}

pub fn read_i64(probe: &[u8], at: usize) -> Option<i64> {
    bytes::<8>(probe, at).map(i64::from_le_bytes)
}

pub fn read_u64(probe: &[u8], at: usize) -> Option<u64> {
    bytes::<8>(probe, at).map(u64::from_le_bytes)
}

/// Whether the first `FRAME_PROBE` bytes of a frame fresh from
/// `av_frame_alloc` at `address` show its defaults where `layout` expects
/// them: `extended_data` points at `data`, `format` -1, the times
/// `AV_NOPTS_VALUE`, `sample_aspect_ratio` and `time_base` 0/1, the colour
/// fields unspecified (primaries, transfer, space 2; range 0).
pub fn fresh_frame_ok(probe: &[u8], address: u64) -> bool {
    use layout::*;
    let i32_at = |at| read_i32(probe, at);
    let i64_at = |at| read_i64(probe, at);
    probe.len() >= FRAME_PROBE
        && read_u64(probe, FRAME_EXTENDED_DATA) == Some(address)
        && (0..8).all(|plane| read_u64(probe, FRAME_DATA + 8 * plane) == Some(0))
        && (0..8).all(|plane| i32_at(FRAME_LINESIZE + 4 * plane) == Some(0))
        && i32_at(FRAME_WIDTH) == Some(0)
        && i32_at(FRAME_HEIGHT) == Some(0)
        && i32_at(FRAME_FORMAT) == Some(-1)
        && i32_at(FRAME_SAMPLE_ASPECT) == Some(0)
        && i32_at(FRAME_SAMPLE_ASPECT + 4) == Some(1)
        && i64_at(FRAME_PTS) == Some(NOPTS)
        && i64_at(FRAME_PKT_DTS) == Some(NOPTS)
        && i32_at(FRAME_TIME_BASE) == Some(0)
        && i32_at(FRAME_TIME_BASE + 4) == Some(1)
        && i32_at(FRAME_COLOR_RANGE) == Some(0)
        && i32_at(FRAME_COLOR_PRIMARIES) == Some(2)
        && i32_at(FRAME_COLOR_TRC) == Some(2)
        && i32_at(FRAME_COLORSPACE) == Some(2)
        && i64_at(FRAME_BEST_EFFORT) == Some(NOPTS)
}

/// Whether a packet fresh from `av_packet_alloc` shows its defaults: no
/// buffer and no data, `pts` and `dts` `AV_NOPTS_VALUE`, `pos` -1,
/// `time_base` 0/1.
pub fn fresh_packet_ok(probe: &[u8]) -> bool {
    use layout::*;
    probe.len() >= PACKET_PROBE
        && read_u64(probe, PACKET_BUF) == Some(0)
        && read_i64(probe, PACKET_PTS) == Some(NOPTS)
        && read_i64(probe, PACKET_DTS) == Some(NOPTS)
        && read_u64(probe, PACKET_DATA) == Some(0)
        && read_i32(probe, PACKET_SIZE) == Some(0)
        && read_i32(probe, PACKET_FLAGS) == Some(0)
        && read_i64(probe, PACKET_POS) == Some(-1)
        && read_i32(probe, PACKET_TIME_BASE) == Some(0)
        && read_i32(probe, PACKET_TIME_BASE + 4) == Some(1)
}

/// The offset of `AVCodecContext.hw_device_ctx` from the run-time offsets
/// of the options `hwaccel_flags` and `extra_hw_frames`: the pointer stands
/// right before `hwaccel_flags`, which `extra_hw_frames` follows
/// (`avcodec.h:1487-1510`). `None` when the DLL's context has another order.
pub fn hw_device_offset(hwaccel_flags: usize, extra_hw_frames: usize) -> Option<usize> {
    (extra_hw_frames == hwaccel_flags + 4 && hwaccel_flags % 8 == 0 && hwaccel_flags >= 16)
        .then(|| hwaccel_flags - 8)
}
