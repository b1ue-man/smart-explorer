//! The Windows side of the virtual display at use time (plan 2026-11-17
//! P5). The driver of VirtualDrivers (installed by the elevated step,
//! `virtual_display_install.rs`) keeps one monitor; se attaches it to the
//! desktop in the listed mode that serves the viewer
//! (`windows_driver::best_mode`), makes it the primary monitor so a
//! session's source (`None` = primary) shows it, and keeps the other
//! displays to its right (`layout_with_virtual`). After the last session the
//! desktop gets its own layout back and the virtual monitor leaves it, unless
//! it is the only display (a headless host keeps it). The layout before the
//! first change is kept in `<app data>\screen\virtual-display-layout.json`,
//! so a host process that ended during a session gives it back at its next
//! start. A viewer's fixed size where the physical monitor shows (P9,
//! `set_display.rs`) sets the mode that monitor lists nearest to it
//! (`physical_mode`); the saved layout gives it its size and rate back.
//! Everything runs in the user's session without administrator rights
//! (display settings are the user's own); nothing here installs or
//! configures the driver.
use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::POINTL;
use windows_sys::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplaySettingsW, CDS_NORESET,
    CDS_SET_PRIMARY, CDS_UPDATEREGISTRY, DEVMODEW, DEVMODEW_0, DEVMODEW_0_1, DISPLAY_DEVICEW,
    DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISPLAY_DEVICE_PRIMARY_DEVICE, DISP_CHANGE_SUCCESSFUL,
    DM_DISPLAYFREQUENCY, DM_PELSHEIGHT, DM_PELSWIDTH, DM_POSITION, ENUM_CURRENT_SETTINGS,
};

use super::device;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::virtual_display::windows_driver::{
    adapter_is_ours, best_mode, layout_with_virtual, physical_monitor, AdapterFact, ListedMode,
    Placement, REASON_DRIVER_BROKEN, REASON_DRIVER_MISSING, REASON_MONITOR_MISSING,
};
use crate::screen::core::virtual_display::DriverLimit;
use crate::screen::core::virtual_display_host::{
    DisplayProbe, SetupProgress, VirtualDisplayDriver,
};

/// The virtual display of a Windows host.
pub struct WindowsVirtualDisplay;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn text(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

fn display_device() -> DISPLAY_DEVICEW {
    // SAFETY: a plain C structure; all zero is a valid value.
    let mut device: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
    device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
    device
}

fn dev_mode() -> DEVMODEW {
    // SAFETY: a plain C structure; all zero is a valid value.
    let mut mode: DEVMODEW = unsafe { std::mem::zeroed() };
    mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
    mode
}

/// One output of a display adapter (`\\.\DISPLAYn`).
struct Output {
    name: String,
    fact: AdapterFact,
    primary: bool,
}

fn outputs() -> Vec<Output> {
    let mut found = Vec::new();
    for index in 0.. {
        let mut adapter = display_device();
        // SAFETY: the structure has its size set.
        if unsafe { EnumDisplayDevicesW(std::ptr::null(), index, &mut adapter, 0) } == 0 {
            break;
        }
        let name = text(&adapter.DeviceName);
        let mut monitor = display_device();
        let wide_name = wide(&name);
        // SAFETY: the name is NUL-terminated; the structure has its size set.
        let has_monitor =
            unsafe { EnumDisplayDevicesW(wide_name.as_ptr(), 0, &mut monitor, 0) } != 0;
        found.push(Output {
            fact: AdapterFact {
                device_id: text(&adapter.DeviceID),
                description: text(&adapter.DeviceString),
                attached: adapter.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0,
                monitor: has_monitor,
            },
            primary: adapter.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE != 0,
            name,
        });
    }
    found
}

fn ours(outputs: &[Output]) -> Option<&Output> {
    outputs
        .iter()
        .find(|output| adapter_is_ours(&output.fact.device_id, &output.fact.description))
}

fn current(name: &str) -> Option<DEVMODEW> {
    let wide_name = wide(name);
    let mut mode = dev_mode();
    // SAFETY: the name is NUL-terminated; the structure has its size set.
    (unsafe { EnumDisplaySettingsW(wide_name.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) } != 0)
        .then_some(mode)
}

/// The modes the virtual monitor offers.
fn listed_modes(name: &str) -> Vec<ListedMode> {
    let wide_name = wide(name);
    let mut modes = Vec::new();
    for index in 0.. {
        let mut mode = dev_mode();
        // SAFETY: as in `current`.
        if unsafe { EnumDisplaySettingsW(wide_name.as_ptr(), index, &mut mode) } == 0 {
            break;
        }
        let listed = ListedMode {
            size: FrameSize::new(mode.dmPelsWidth, mode.dmPelsHeight),
            refresh_hz: mode.dmDisplayFrequency,
        };
        if !modes.contains(&listed) {
            modes.push(listed);
        }
    }
    modes
}

/// The displays that are part of the desktop now.
fn placements(outputs: &[Output]) -> Vec<Placement> {
    outputs
        .iter()
        .filter(|output| output.fact.attached)
        .filter_map(|output| {
            let mode = current(&output.name)?;
            // SAFETY: a display's current settings carry the position.
            let position = unsafe { mode.Anonymous1.Anonymous2.dmPosition };
            Some(Placement {
                device: output.name.clone(),
                x: position.x,
                y: position.y,
                width: mode.dmPelsWidth,
                height: mode.dmPelsHeight,
                primary: output.primary,
                refresh_hz: mode.dmDisplayFrequency,
            })
        })
        .collect()
}

fn layout_file() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("screen")
        .join("virtual-display-layout.json")
}

fn saved_layout() -> Option<Vec<Placement>> {
    let bytes = std::fs::read(layout_file()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_layout(layout: &[Placement]) -> io::Result<()> {
    let path = layout_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(layout).map_err(io::Error::other)?;
    std::fs::write(path, bytes)
}

/// Stages one display's settings (applied by `commit`).
fn stage(name: &str, mode: &DEVMODEW, primary: bool) -> io::Result<()> {
    let wide_name = wide(name);
    let mut flags = CDS_UPDATEREGISTRY | CDS_NORESET;
    if primary {
        flags |= CDS_SET_PRIMARY;
    }
    // SAFETY: the name is NUL-terminated and the mode has its size set.
    let result = unsafe {
        ChangeDisplaySettingsExW(
            wide_name.as_ptr(),
            mode,
            std::ptr::null_mut(),
            flags,
            std::ptr::null(),
        )
    };
    if result == DISP_CHANGE_SUCCESSFUL {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{name}: display change refused ({result})"
        )))
    }
}

/// Applies every staged change at once.
fn commit() -> io::Result<()> {
    // SAFETY: the documented call that applies the staged registry settings.
    let result = unsafe {
        ChangeDisplaySettingsExW(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
        )
    };
    if result == DISP_CHANGE_SUCCESSFUL {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "display layout refused ({result})"
        )))
    }
}

fn positioned(x: i32, y: i32, size: Option<(u32, u32, u32)>) -> DEVMODEW {
    let mut mode = dev_mode();
    mode.dmFields = DM_POSITION;
    mode.Anonymous1 = DEVMODEW_0 {
        Anonymous2: DEVMODEW_0_1 {
            dmPosition: POINTL { x, y },
            dmDisplayOrientation: 0,
            dmDisplayFixedOutput: 0,
        },
    };
    if let Some((width, height, refresh)) = size {
        mode.dmFields |= DM_PELSWIDTH | DM_PELSHEIGHT;
        mode.dmPelsWidth = width;
        mode.dmPelsHeight = height;
        if refresh > 0 {
            mode.dmFields |= DM_DISPLAYFREQUENCY;
            mode.dmDisplayFrequency = refresh;
        }
    }
    mode
}

fn not_found(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, reason.to_string())
}

/// Gives the desktop the layout before se's first change back: every
/// other display its place, size and rate, the virtual monitor off it.
fn restore_layout() -> io::Result<()> {
    let Some(original) = saved_layout() else {
        return Ok(());
    };
    let found = outputs();
    let virtual_name = ours(&found).map(|output| output.name.clone());
    let others: Vec<&Placement> = original
        .iter()
        .filter(|p| Some(&p.device) != virtual_name.as_ref())
        .collect();
    if !others.is_empty() {
        for placement in &others {
            let size = Some((placement.width, placement.height, placement.refresh_hz))
                .filter(|(width, height, _)| *width > 0 && *height > 0);
            stage(
                &placement.device,
                &positioned(placement.x, placement.y, size),
                placement.primary,
            )?;
        }
        if let Some(name) = &virtual_name {
            // Width and height 0 take a display off the desktop.
            stage(name, &positioned(0, 0, Some((0, 0, 0))), false)?;
        }
        commit()?;
    }
    std::fs::remove_file(layout_file()).or_else(|error| match error.kind() {
        io::ErrorKind::NotFound => Ok(()),
        _ => Err(error),
    })
}

/// A host process gives back a layout a former process left (once).
fn restore_left_layout() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        if let Err(error) = restore_layout() {
            eprintln!("virtual display: former layout: {error}");
        }
    });
}

/// After the driver was installed: the new monitor leaves the desktop again
/// where a physical monitor is there (the elevated step runs while the
/// user works).
pub fn release_idle() {
    let found = outputs();
    let Some(virtual_output) = ours(&found) else {
        return;
    };
    let facts: Vec<AdapterFact> = found.iter().map(|output| output.fact.clone()).collect();
    if virtual_output.fact.attached && physical_monitor(&facts) && saved_layout().is_none() {
        let detached = stage(
            &virtual_output.name,
            &positioned(0, 0, Some((0, 0, 0))),
            false,
        )
        .and_then(|()| commit());
        if let Err(error) = detached {
            eprintln!("virtual display: detach after setup: {error}");
        }
    }
}

impl VirtualDisplayDriver for WindowsVirtualDisplay {
    fn probe(&self) -> io::Result<DisplayProbe> {
        restore_left_layout();
        let found = outputs();
        let facts: Vec<AdapterFact> = found.iter().map(|output| output.fact.clone()).collect();
        let virtual_output = ours(&found);
        let unsupported = if device::find().is_none() {
            Some(REASON_DRIVER_MISSING)
        } else if !device::working() {
            Some(REASON_DRIVER_BROKEN)
        } else if virtual_output.is_none() {
            Some(REASON_MONITOR_MISSING)
        } else {
            None
        };
        let current = virtual_output
            .filter(|output| output.fact.attached)
            .and_then(|output| current(&output.name))
            .map(|mode| FrameSize::new(mode.dmPelsWidth, mode.dmPelsHeight));
        Ok(DisplayProbe {
            physical_monitor: physical_monitor(&facts),
            modes: unsupported.is_none(),
            limit: DriverLimit::NONE,
            driver: "vdd".into(),
            current,
            unsupported: unsupported.map(str::to_string),
            // Never a setup step during use (user 2026-11-17): the driver
            // comes with the installation or an update.
            setup_possible: false,
            portal: false,
        })
    }

    fn apply(&self, size: FrameSize) -> io::Result<FrameSize> {
        let found = outputs();
        let virtual_output = ours(&found).ok_or_else(|| not_found(REASON_MONITOR_MISSING))?;
        let mode = best_mode(size, &listed_modes(&virtual_output.name))
            .ok_or_else(|| not_found("the virtual monitor lists no mode"))?;
        let original = match saved_layout() {
            Some(original) => original,
            None => {
                let original = placements(&found);
                save_layout(&original)?;
                original
            }
        };
        for placement in layout_with_virtual(&original, &virtual_output.name, mode.size) {
            let own = placement.device == virtual_output.name;
            let size = own.then_some((mode.size.width, mode.size.height, mode.refresh_hz));
            stage(
                &placement.device,
                &positioned(placement.x, placement.y, size),
                placement.primary,
            )?;
        }
        commit()?;
        let now = current(&virtual_output.name)
            .ok_or_else(|| io::Error::other("the virtual monitor did not join the desktop"))?;
        Ok(FrameSize::new(now.dmPelsWidth, now.dmPelsHeight))
    }

    fn request_setup(&self) -> SetupProgress {
        SetupProgress::Failed(REASON_DRIVER_MISSING.into())
    }

    /// The physical display that shows the desktop (the primary one that is
    /// not se's, else the first attached one) in the mode it lists nearest
    /// to `size`; se's virtual monitor leaves the desktop first.
    fn physical_mode(&self, size: FrameSize) -> io::Result<FrameSize> {
        // Gemini round 3, 22: only se's virtual monitor on the desktop needs
        // the layout back first; a second physical mode keeps the saved
        // original and switches the display once, not back and then on.
        if ours(&outputs()).is_some_and(|output| output.fact.attached) {
            restore_layout()?;
        }
        let found = outputs();
        let physical = found
            .iter()
            .filter(|output| {
                output.fact.attached
                    && !adapter_is_ours(&output.fact.device_id, &output.fact.description)
            })
            .max_by_key(|output| output.primary)
            .ok_or_else(|| not_found("no physical display on the desktop"))?;
        let mode = best_mode(size, &listed_modes(&physical.name))
            .ok_or_else(|| not_found("the physical display lists no mode"))?;
        let now = current(&physical.name)
            .ok_or_else(|| io::Error::other("the physical display has no current mode"))?;
        if saved_layout().is_none() {
            save_layout(&placements(&found))?;
        }
        // SAFETY: a display's current settings carry the position.
        let position = unsafe { now.Anonymous1.Anonymous2.dmPosition };
        let wanted = (mode.size.width, mode.size.height, mode.refresh_hz);
        stage(
            &physical.name,
            &positioned(position.x, position.y, Some(wanted)),
            physical.primary,
        )?;
        commit()?;
        let shown = current(&physical.name)
            .ok_or_else(|| io::Error::other("the physical display left the desktop"))?;
        Ok(FrameSize::new(shown.dmPelsWidth, shown.dmPelsHeight))
    }

    /// After the last session: the desktop's own layout; the virtual monitor
    /// leaves it unless it is the only display.
    fn restore(&self) -> io::Result<()> {
        restore_layout()
    }
}
