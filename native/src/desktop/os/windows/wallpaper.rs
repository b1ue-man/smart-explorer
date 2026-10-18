//! The system wallpaper of one monitor on Windows (DSK-M5, C1.2, R25, DN5):
//! the image per monitor through `IDesktopWallpaper::GetWallpaper(<monitor
//! ID>)`, never with `NULL`; fit mode (`GetPosition`) and background color
//! (`GetBackgroundColor`) are system-wide. While a slideshow runs
//! (`GetStatus` with `DSS_SLIDESHOW`) the surface reads again in the
//! system's own interval (`GetSlideshowOptions`) and on
//! `SPI_SETDESKWALLPAPER` (Annahme A10). An empty path (solid color, Windows
//! Spotlight) shows the system color (Annahme A9); so does a monitor the
//! interface does not know.
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::time::Duration;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};
use windows::Win32::UI::Shell::{
    DesktopWallpaper, IDesktopWallpaper, DESKTOP_SLIDESHOW_OPTIONS, DSS_SLIDESHOW,
};

use super::monitors::{self, MonitorInfo};
use super::ComApartment;
use crate::app::desktop_surface_host::SystemBackground;
use crate::desktop::os::wallpaper_image::{FitMode, SystemWallpaper, DEFAULT_COLOR};

/// What `IDesktopWallpaper` answers; the task test puts a fake in its place.
pub(crate) trait WallpaperSource {
    /// Every monitor ID with its rectangle (`GetMonitorDevicePathAt`,
    /// `GetMonitorRECT`); a monitor without rectangle is not attached.
    fn monitor_ids(&self) -> Vec<(String, Option<[i32; 4]>)>;
    /// `GetWallpaper(<monitor ID>)`; empty for a solid color or Spotlight.
    fn wallpaper(&self, monitor_id: &str) -> Result<String, String>;
    /// `GetPosition()` (`DESKTOP_WALLPAPER_POSITION`).
    fn position(&self) -> Result<i32, String>;
    /// `GetBackgroundColor()` as COLORREF `0x00BBGGRR`.
    fn background_color(&self) -> Result<u32, String>;
    /// The slideshow's interval while one is configured.
    fn slideshow_tick(&self) -> Option<Duration>;
}

/// COLORREF `0x00BBGGRR` as RGB.
pub(crate) fn rgb(colorref: u32) -> [u8; 3] {
    let [red, green, blue, _] = colorref.to_le_bytes();
    [red, green, blue]
}

/// The interface's ID of `monitor`: its device path in any letter case,
/// else the attached ID with the same rectangle, else the only one.
pub(crate) fn monitor_id(source: &dyn WallpaperSource, monitor: &MonitorInfo) -> Option<String> {
    let ids = source.monitor_ids();
    if let Some(path) = &monitor.device_path {
        if let Some((id, _)) = ids.iter().find(|(id, _)| id.eq_ignore_ascii_case(path)) {
            return Some(id.clone());
        }
    }
    let attached: Vec<&(String, Option<[i32; 4]>)> =
        ids.iter().filter(|(_, rect)| rect.is_some()).collect();
    if let Some((id, _)) = attached
        .iter()
        .find(|(_, rect)| *rect == Some(monitor.rect))
    {
        return Some(id.clone());
    }
    match attached.as_slice() {
        [(id, _)] => Some(id.clone()),
        _ => None,
    }
}

/// The background of `monitor` as `source` describes it. Only a source
/// that names neither image, fit mode nor color fails; a failed part keeps
/// the default (fit mode "Füllen", color `DEFAULT_COLOR`, no image).
pub(crate) fn background_of(
    source: &dyn WallpaperSource,
    monitor: &MonitorInfo,
) -> Result<SystemBackground, String> {
    let color = source.background_color();
    let position = source.position();
    let image = match monitor_id(source, monitor) {
        Some(id) => source.wallpaper(&id),
        None => Err(format!(
            "{} ist dem Hintergrund unbekannt",
            monitor.gdi_name
        )),
    };
    if let (Err(error), Err(_), Err(_)) = (&image, &color, &position) {
        return Err(error.clone());
    }
    let path = image
        .ok()
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from);
    Ok(SystemBackground {
        wallpaper: SystemWallpaper {
            path,
            mode: position.map_or(FitMode::Fill, |position| {
                FitMode::from_windows(u32::try_from(position).unwrap_or(u32::MAX))
            }),
            color: color.map_or(DEFAULT_COLOR, rgb),
        },
        refresh_every: source.slideshow_tick(),
    })
}

/// `IDesktopWallpaper` of this session.
struct ComSource {
    wallpaper: IDesktopWallpaper,
}

fn owned(text: PWSTR) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: the interface returns a NUL-terminated string it allocated
    // with the task allocator.
    let value = unsafe { OsString::from_wide(text.as_wide()) };
    unsafe { CoTaskMemFree(Some(text.0 as *const std::ffi::c_void)) };
    value.to_string_lossy().into_owned()
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

impl WallpaperSource for ComSource {
    fn monitor_ids(&self) -> Vec<(String, Option<[i32; 4]>)> {
        let count = unsafe { self.wallpaper.GetMonitorDevicePathCount() }.unwrap_or(0);
        (0..count)
            .filter_map(|index| unsafe { self.wallpaper.GetMonitorDevicePathAt(index) }.ok())
            .map(owned)
            .filter(|id| !id.is_empty())
            .map(|id| {
                let name = wide(&id);
                let rect = unsafe { self.wallpaper.GetMonitorRECT(PCWSTR(name.as_ptr())) }
                    .ok()
                    .map(|rect| [rect.left, rect.top, rect.right, rect.bottom]);
                (id, rect)
            })
            .collect()
    }

    fn wallpaper(&self, monitor_id: &str) -> Result<String, String> {
        let name = wide(monitor_id);
        unsafe { self.wallpaper.GetWallpaper(PCWSTR(name.as_ptr())) }
            .map(owned)
            .map_err(|error| format!("GetWallpaper: {error}"))
    }

    fn position(&self) -> Result<i32, String> {
        unsafe { self.wallpaper.GetPosition() }
            .map(|position| position.0)
            .map_err(|error| format!("GetPosition: {error}"))
    }

    fn background_color(&self) -> Result<u32, String> {
        unsafe { self.wallpaper.GetBackgroundColor() }
            .map(|color| color.0)
            .map_err(|error| format!("GetBackgroundColor: {error}"))
    }

    fn slideshow_tick(&self) -> Option<Duration> {
        let status = unsafe { self.wallpaper.GetStatus() }.ok()?;
        if status.0 & DSS_SLIDESHOW.0 == 0 {
            return None;
        }
        let mut options = DESKTOP_SLIDESHOW_OPTIONS::default();
        let mut tick = 0u32;
        unsafe { self.wallpaper.GetSlideshowOptions(&mut options, &mut tick) }.ok()?;
        (tick > 0).then(|| Duration::from_millis(u64::from(tick)))
    }
}

/// The system background of the monitor with `monitor_key`, read now.
pub(crate) fn system_wallpaper(monitor_key: &str) -> Result<SystemBackground, String> {
    let monitor = monitors::info_for(monitor_key)
        .ok_or_else(|| format!("Bildschirm {monitor_key} ist nicht angeschlossen"))?;
    let _apartment = ComApartment::enter();
    let wallpaper: IDesktopWallpaper =
        unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL) }
            .map_err(|error| format!("IDesktopWallpaper: {error}"))?;
    background_of(&ComSource { wallpaper }, &monitor)
}
