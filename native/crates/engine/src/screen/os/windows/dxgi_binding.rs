//! The output binding of a duplication for the verified login helper (moved
//! out of `capture_dxgi.rs` unchanged for plan 2026-11-17 P9, which keeps
//! that file under 500 lines).
use std::io;

use super::DxgiCapture;
use crate::screen::core::pixel_convert::FrameSize;

/// Geometry published to the verified login helper's existing input guard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::screen::os) struct OutputBinding {
    pub device: String,
    pub monitor: usize,
    pub area: crate::screen::os::input_send::Area,
    pub rotation: i32,
    pub duplication_rotation: i32,
    pub generation: u64,
    pub size: FrameSize,
    pub attached: bool,
}
impl DxgiCapture {
    pub(in crate::screen::os) fn output_binding(&self) -> io::Result<OutputBinding> {
        let output = unsafe { self.prepared.output.GetDesc() }.map_err(io::Error::other)?;
        let duplication = unsafe { self.live_duplication()?.GetDesc() };
        let end = output
            .DeviceName
            .iter()
            .position(|u| *u == 0)
            .unwrap_or(output.DeviceName.len());
        let r = output.DesktopCoordinates;
        let area = crate::screen::os::input_send::Area {
            left: r.left,
            top: r.top,
            width: r
                .right
                .checked_sub(r.left)
                .filter(|n| *n > 0)
                .ok_or_else(|| io::Error::other("DXGI output width unavailable"))?,
            height: r
                .bottom
                .checked_sub(r.top)
                .filter(|n| *n > 0)
                .ok_or_else(|| io::Error::other("DXGI output height unavailable"))?,
        };
        Ok(OutputBinding {
            device: String::from_utf16_lossy(&output.DeviceName[..end]),
            monitor: output.Monitor.0 as usize,
            area,
            rotation: output.Rotation.0,
            duplication_rotation: duplication.Rotation.0,
            generation: self.budget.generation(),
            size: self.size,
            attached: output.AttachedToDesktop.as_bool(),
        })
    }
}
