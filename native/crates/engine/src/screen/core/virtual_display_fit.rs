//! Fitting a display size to the limit of the display driver and the mode
//! timings of se's display modes (plan 2026-11-17 P5, decision E2): the
//! aspect ratio kept, the width a multiple of 8, at most 16384 per side and
//! what the driver and se's capture take; timings by CVT reduced blanking
//! as `cvt -r` computes them.
use std::io;

use super::super::frame_budget::FrameLimits;
use super::super::pixel_convert::FrameSize;
use super::super::wire_hello::MAX_DISPLAY_SIDE;
use super::{MIN_HEIGHT, MIN_WIDTH, WIDTH_STEP};

/// The largest display a driver takes: sides and pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverLimit {
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
}

impl DriverLimit {
    /// What every display of se meets: 16384 per side (the largest texture
    /// side of current GPUs, `MAX_DISPLAY_SIDE`) and the pixels se's capture
    /// takes (`FrameLimits::RESOURCE`: a larger desktop would end the
    /// session with `source_exceeds_frame_limit`).
    pub const NONE: Self = Self {
        max_width: MAX_DISPLAY_SIDE,
        max_height: MAX_DISPLAY_SIDE,
        max_pixels: FrameLimits::RESOURCE.max_pixels,
    };

    /// The X server's screen size range.
    pub fn sides(max_width: u32, max_height: u32) -> Self {
        Self {
            max_width: max_width.min(MAX_DISPLAY_SIDE),
            max_height: max_height.min(MAX_DISPLAY_SIDE),
            ..Self::NONE
        }
    }

    /// bochs-drm under `modesetting`: a mode must fit into half of the video
    /// memory (two buffers of 32-bit pixels).
    pub fn bochs(video_memory_bytes: u64) -> Self {
        Self {
            max_pixels: video_memory_bytes / 2 / 4,
            ..Self::NONE
        }
    }

    /// The Xorg driver `dummy`: one 32-bit framebuffer in `VideoRam` KiB.
    pub fn dummy(video_ram_kib: u64) -> Self {
        Self {
            max_pixels: video_ram_kib.saturating_mul(1024) / 4,
            ..Self::NONE
        }
    }

    /// Both limits at once.
    pub fn narrowed(self, other: Self) -> Self {
        Self {
            max_width: self.max_width.min(other.max_width),
            max_height: self.max_height.min(other.max_height),
            max_pixels: self.max_pixels.min(other.max_pixels),
        }
    }
}

/// The display size for `wanted` within `limit` (E2): the aspect ratio
/// kept, the width a multiple of 8, at least 640 × 360 (enlarged with the
/// same aspect ratio), at most what `DriverLimit::NONE` and the driver
/// take. Integer arithmetic only, so the result is the same on every host.
/// `None` for an empty size or a limit below 8 pixels of width.
pub fn fit_mode(wanted: FrameSize, limit: DriverLimit) -> Option<FrameSize> {
    let (w, h) = (u64::from(wanted.width), u64::from(wanted.height));
    if w == 0 || h == 0 {
        return None;
    }
    let limit = limit.narrowed(DriverLimit::NONE);
    let max_w = u64::from(limit.max_width);
    let max_h = u64::from(limit.max_height);
    let height_for = |width: u64| width * h / w;
    // Enlarged to the minimum, kept otherwise.
    let up = w
        .max(u64::from(MIN_WIDTH))
        .max((u64::from(MIN_HEIGHT) * w).div_ceil(h));
    let by_height = max_h * w / h;
    let by_pixels = (limit.max_pixels.saturating_mul(w) / h).isqrt();
    let step = u64::from(WIDTH_STEP);
    let mut width = up.min(max_w).min(by_height).min(by_pixels) / step * step;
    while width >= step {
        let height = height_for(width);
        if (1..=max_h).contains(&height) && width * height <= limit.max_pixels {
            return Some(FrameSize::new(width as u32, height as u32));
        }
        width -= step;
    }
    None
}

/// One display mode: pixel clock (kHz) and the horizontal and vertical
/// timings of the X11 mode line. CVT reduced blanking has `+hsync -vsync`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeTiming {
    pub clock_khz: u32,
    pub hdisplay: u32,
    pub hsync_start: u32,
    pub hsync_end: u32,
    pub htotal: u32,
    pub vdisplay: u32,
    pub vsync_start: u32,
    pub vsync_end: u32,
    pub vtotal: u32,
}

impl ModeTiming {
    /// The Xorg `Modeline` of the mode (`cvt -r` prints the same numbers).
    pub fn modeline(&self, name: &str) -> String {
        format!(
            "Modeline \"{name}\" {}.{:02} {} {} {} {} {} {} {} {} +hsync -vsync",
            self.clock_khz / 1000,
            (self.clock_khz % 1000) / 10,
            self.hdisplay,
            self.hsync_start,
            self.hsync_end,
            self.htotal,
            self.vdisplay,
            self.vsync_start,
            self.vsync_end,
            self.vtotal,
        )
    }
}

/// The CVT 1.2 vertical sync width for the aspect ratio of the mode.
fn cvt_vsync(width: u32, height: u32) -> u32 {
    if height % 3 == 0 && height * 4 / 3 == width {
        4
    } else if height % 9 == 0 && height * 16 / 9 == width {
        5
    } else if height % 10 == 0 && height * 16 / 10 == width {
        6
    } else if (height % 4 == 0 && height * 5 / 4 == width)
        || (height % 9 == 0 && height * 15 / 9 == width)
    {
        7
    } else {
        10
    }
}

/// CVT reduced blanking (VESA CVT 1.2 as `libxcvt_gen_mode_info` and the
/// X server's `xf86CVTMode` compute it, which `cvt -r` prints): 160 pixels
/// horizontal blank, 32 pixels sync, 460 µs minimum vertical blank, 3 lines
/// front porch, clock in steps of 250 kHz. The horizontal period is a
/// `float` there, so it is an `f32` here: the same numbers on every size.
/// 1920 × 1080 @ 60 gives 138.50 MHz, 1920 1968 2000 2080, 1080 1083 1088 1111.
pub fn cvt_rb(width: u32, height: u32, refresh_hz: u32) -> io::Result<ModeTiming> {
    const RB_MIN_VBLANK_US: f64 = 460.0;
    const RB_H_SYNC: u32 = 32;
    const RB_H_BLANK: u32 = 160;
    const RB_V_FPORCH: u32 = 3;
    const MIN_V_BPORCH: u32 = 6;
    const CLOCK_STEP_KHZ: u32 = 250;
    let invalid = |text: &str| io::Error::new(io::ErrorKind::InvalidInput, text.to_string());
    if width < WIDTH_STEP || height == 0 || refresh_hz == 0 || height > MAX_DISPLAY_SIDE {
        return Err(invalid("display mode is empty or too large"));
    }
    let hdisplay = width - width % WIDTH_STEP;
    let vsync = cvt_vsync(width, height);
    // HPeriod = (float)(1000000.0 / VFieldRate - CVT_RB_MIN_VBLANK) / VDisplayRnd
    let field_us = 1_000_000.0 / f64::from(refresh_hz as f32);
    let hperiod_us = (field_us - RB_MIN_VBLANK_US) as f32 / height as f32;
    if hperiod_us.is_nan() || hperiod_us <= 0.0 {
        return Err(invalid("display mode is too tall for its refresh rate"));
    }
    // VBILines = (float)CVT_RB_MIN_VBLANK / HPeriod + 1, truncated.
    let vbi_lines = ((RB_MIN_VBLANK_US as f32 / hperiod_us + 1.0) as u32)
        .max(RB_V_FPORCH + vsync + MIN_V_BPORCH);
    let vtotal = height + vbi_lines;
    let htotal = hdisplay + RB_H_BLANK;
    // Clock = HTotal * 1000.0 / HPeriod (double), truncated, then stepped.
    let clock = (f64::from(htotal) * 1000.0 / f64::from(hperiod_us)) as u32;
    let clock_khz = clock - clock % CLOCK_STEP_KHZ;
    let hsync_end = hdisplay + RB_H_BLANK / 2;
    let vsync_start = height + RB_V_FPORCH;
    Ok(ModeTiming {
        clock_khz,
        hdisplay,
        hsync_start: hsync_end - RB_H_SYNC,
        hsync_end,
        htotal,
        vdisplay: height,
        vsync_start,
        vsync_end: vsync_start + vsync,
        vtotal,
    })
}
