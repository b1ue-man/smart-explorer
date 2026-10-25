//! Wallpaper of a surface (DSK-M4): what the configured or the system
//! background means for one monitor, decoding of the image (`image`, formats
//! PNG, JPEG, GIF, WebP and BMP, H6) and the six fit modes of Windows and
//! the Linux desktops: Füllen, Einpassen, Strecken, Kacheln, Zentrieren,
//! Spannen. A background without a decodable image shows its color: solid
//! colors and Windows Spotlight give no path, HDR wallpapers a format
//! without decoder (Annahme A9); neither is an error.
//!
//! The geometry works in points of the surface; the view turns it into egui
//! shapes. DSK-M5 and DSK-M6 deliver the system wallpaper as
//! [`SystemWallpaper`] (path, fit mode, color of the monitor).
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

#[path = "wallpaper_source.rs"]
mod source;

use crate::desktop::core::config::{Background, BackgroundKind};

/// Shown behind and instead of a wallpaper when the system names no color.
pub(crate) const DEFAULT_COLOR: [u8; 3] = [0x1F, 0x2A, 0x33];
/// Larger images are refused before decoding (memory bound).
const MAX_DECODE_SIDE: u32 = 16_384;

/// How an image covers its monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FitMode {
    /// Scaled to cover the monitor, cropped in the middle.
    Fill,
    /// Scaled to lie inside the monitor, the color around it.
    Fit,
    /// Scaled to the monitor without keeping its proportions.
    Stretch,
    /// In its own size, repeated from the top left.
    Tile,
    /// In its own size in the middle, the color around it.
    Center,
    /// One image over all monitors, scaled like `Fill`.
    Span,
}

impl FitMode {
    /// Windows `DESKTOP_WALLPAPER_POSITION` (`IDesktopWallpaper::GetPosition`).
    pub(crate) fn from_windows(position: u32) -> FitMode {
        match position {
            0 => FitMode::Center,
            1 => FitMode::Tile,
            2 => FitMode::Stretch,
            3 => FitMode::Fit,
            5 => FitMode::Span,
            _ => FitMode::Fill,
        }
    }

    /// GNOME, Cinnamon and MATE `picture-options`.
    pub(crate) fn from_gnome(option: &str) -> FitMode {
        match option {
            "wallpaper" => FitMode::Tile,
            "centered" => FitMode::Center,
            "scaled" => FitMode::Fit,
            "stretched" => FitMode::Stretch,
            "spanned" => FitMode::Span,
            _ => FitMode::Fill,
        }
    }
}

/// The system's wallpaper of one monitor (DSK-M5, DSK-M6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SystemWallpaper {
    /// `None` or empty: a solid color or a source without file.
    pub(crate) path: Option<PathBuf>,
    pub(crate) mode: FitMode,
    pub(crate) color: [u8; 3],
}

/// What one surface paints behind its tiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WallpaperSpec {
    /// Nothing: the desktop below shows through (`transparent`).
    Transparent,
    Color([u8; 3]),
    /// An image in its fit mode with the color around and instead of it.
    Image {
        path: PathBuf,
        mode: FitMode,
        color: [u8; 3],
    },
}

/// The spec of a configured background; `system` is the platform's wallpaper
/// of this monitor while one is known. A configured image is filled in.
pub(crate) fn spec_for(background: &Background, system: Option<&SystemWallpaper>) -> WallpaperSpec {
    let value = background
        .value
        .as_deref()
        .filter(|value| !value.is_empty());
    match background.kind {
        BackgroundKind::Transparent => WallpaperSpec::Transparent,
        BackgroundKind::Color => {
            WallpaperSpec::Color(value.and_then(parse_color).unwrap_or(DEFAULT_COLOR))
        }
        BackgroundKind::Image => match value {
            Some(path) => WallpaperSpec::Image {
                path: PathBuf::from(path),
                mode: FitMode::Fill,
                color: DEFAULT_COLOR,
            },
            None => WallpaperSpec::Color(DEFAULT_COLOR),
        },
        BackgroundKind::System => match system {
            Some(system) => match system
                .path
                .as_ref()
                .filter(|path| !path.as_os_str().is_empty())
            {
                Some(path) => WallpaperSpec::Image {
                    path: path.clone(),
                    mode: system.mode,
                    color: system.color,
                },
                None => WallpaperSpec::Color(system.color),
            },
            None => WallpaperSpec::Color(DEFAULT_COLOR),
        },
    }
}

/// `#RRGGBB` of `config.json` as RGB.
pub(crate) fn parse_color(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// RGB as `#RRGGBB` (`--background color:#RRGGBB`).
pub(crate) fn format_color(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

/// A rectangle in points: top left corner, width and height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Area {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
}

impl Area {
    pub(crate) fn new(x: f32, y: f32, w: f32, h: f32) -> Area {
        Area { x, y, w, h }
    }

    fn centered_in(within: Area, w: f32, h: f32) -> Area {
        Area::new(
            within.x + (within.w - w) / 2.0,
            within.y + (within.h - h) / 2.0,
            w,
            h,
        )
    }

    fn intersect(&self, other: &Area) -> Option<Area> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.w).min(other.x + other.w);
        let bottom = (self.y + self.h).min(other.y + other.h);
        (right > left && bottom > top).then(|| Area::new(left, top, right - left, bottom - top))
    }
}

/// One textured rectangle of the wallpaper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    /// Where the image is drawn on the surface.
    pub(crate) dest: Area,
    /// The part of the image in texture coordinates (beyond 1 repeats).
    pub(crate) uv: Area,
    /// The texture repeats (`Tile`).
    pub(crate) repeat: bool,
}

/// Where an image of `image` pixels lies on `monitor` (points of the
/// surface). `pixels_per_point` turns pixels into points for the modes in the
/// image's own size; `span` is every monitor together in the same points
/// (`Span`; without it `Span` fills the monitor). `None`: nothing to draw.
pub(crate) fn place(
    mode: FitMode,
    image: (u32, u32),
    monitor: Area,
    pixels_per_point: f32,
    span: Option<Area>,
) -> Option<Placement> {
    let (width, height) = (image.0 as f32, image.1 as f32);
    if width <= 0.0 || height <= 0.0 || monitor.w <= 0.0 || monitor.h <= 0.0 {
        return None;
    }
    let ppp = if pixels_per_point > 0.0 {
        pixels_per_point
    } else {
        1.0
    };
    let drawn = match mode {
        FitMode::Stretch => monitor,
        FitMode::Fit => {
            let scale = (monitor.w / width).min(monitor.h / height);
            Area::centered_in(monitor, width * scale, height * scale)
        }
        FitMode::Fill => {
            let scale = (monitor.w / width).max(monitor.h / height);
            Area::centered_in(monitor, width * scale, height * scale)
        }
        FitMode::Center => Area::centered_in(monitor, width / ppp, height / ppp),
        FitMode::Span => {
            let all = span.unwrap_or(monitor);
            let scale = (all.w / width).max(all.h / height);
            Area::centered_in(all, width * scale, height * scale)
        }
        FitMode::Tile => {
            let uv = Area::new(0.0, 0.0, monitor.w * ppp / width, monitor.h * ppp / height);
            return Some(Placement {
                dest: monitor,
                uv,
                repeat: true,
            });
        }
    };
    let dest = drawn.intersect(&monitor)?;
    let uv = Area::new(
        (dest.x - drawn.x) / drawn.w,
        (dest.y - drawn.y) / drawn.h,
        dest.w / drawn.w,
        dest.h / drawn.h,
    );
    Some(Placement {
        dest,
        uv,
        repeat: false,
    })
}

/// Pixels of a decoded wallpaper: RGBA, straight alpha, rows top down.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct DecodedImage {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

impl fmt::Debug for DecodedImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "DecodedImage({}x{})", self.width, self.height)
    }
}

/// The result of loading a wallpaper: its pixels, or why its color shows.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Loaded {
    Image(DecodedImage),
    Color(String),
}

/// Loads the wallpaper at `path`, at most `max_side` pixels on its longer
/// side (the texture limit of the surface). No path, an empty path, a
/// missing or unreadable file and a format without decoder give the color.
pub(crate) fn load(path: Option<&Path>, max_side: u32) -> Loaded {
    load_cancellable(path, max_side, Arc::new(AtomicBool::new(false)))
}

pub(crate) fn load_cancellable(path: Option<&Path>, max_side: u32, cancel: Arc<AtomicBool>) -> Loaded {
    let Some(path) = path.filter(|path| !path.as_os_str().is_empty()) else {
        return Loaded::Color("kein Bild (Volltonfarbe)".into());
    };
    match decode(path, max_side, cancel) {
        Ok(image) => Loaded::Image(image),
        Err(error) => Loaded::Color(error),
    }
}

fn decode(path: &Path, max_side: u32, cancel: Arc<AtomicBool>) -> Result<DecodedImage, String> {
    let shown = |error: &dyn fmt::Display| format!("{}: {error}", path.display());
    let file = source::WallpaperReader::open(path, Arc::clone(&cancel)).map_err(|error| shown(&error))?;
    let mut reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .map_err(|error| shown(&error))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_SIDE);
    limits.max_image_height = Some(MAX_DECODE_SIDE);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|error| shown(&error))?;
    if cancel.load(Ordering::Acquire) { return Err("wallpaper read canceled".into()); }
    let rgba = shrink(decoded.into_rgba8(), max_side);
    if cancel.load(Ordering::Acquire) { return Err("wallpaper read canceled".into()); }
    let (width, height) = rgba.dimensions();
    Ok(DecodedImage {
        width,
        height,
        rgba: rgba.into_raw(),
    })
}

/// Scales an image down to `max_side` on its longer side, keeping its shape.
fn shrink(image: image::RgbaImage, max_side: u32) -> image::RgbaImage {
    let (width, height) = image.dimensions();
    let max_side = max_side.max(1);
    if width <= max_side && height <= max_side {
        return image;
    }
    let scale = f64::from(max_side) / f64::from(width.max(height));
    let side = |value: u32| ((f64::from(value) * scale).round() as u32).clamp(1, max_side);
    image::imageops::resize(
        &image,
        side(width),
        side(height),
        image::imageops::FilterType::Triangle,
    )
}

#[cfg(test)]
#[path = "wallpaper_image_tests.rs"]
mod tests;
