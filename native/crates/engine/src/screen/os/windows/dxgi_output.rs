//! The DXGI output of a monitor and its rotation (moved out of
//! `capture_dxgi.rs` unchanged, BK3b, to keep that file under 500 lines).
use crate::screen::core::pixel_convert::BgraRotation;
use std::io;
use windows::core::Interface;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_MODE_ROTATION, DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_ROTATE180,
    DXGI_MODE_ROTATION_ROTATE270, DXGI_MODE_ROTATION_ROTATE90, DXGI_MODE_ROTATION_UNSPECIFIED,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, IDXGIOutput1,
};

pub(super) fn find_output(device: &str) -> io::Result<(IDXGIAdapter, IDXGIOutput1)> {
    // SAFETY: plain owned DXGI enumeration; no capture surface is opened.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(io::Error::other)?;
        let mut adapter_index = 0;
        while let Ok(adapter) = factory.EnumAdapters1(adapter_index) {
            let mut output_index = 0;
            while let Ok(output) = adapter.EnumOutputs(output_index) {
                let desc = output.GetDesc().map_err(io::Error::other)?;
                let end = desc
                    .DeviceName
                    .iter()
                    .position(|u| *u == 0)
                    .unwrap_or(desc.DeviceName.len());
                if String::from_utf16_lossy(&desc.DeviceName[..end]) == device {
                    return Ok((
                        adapter.cast().map_err(io::Error::other)?,
                        output.cast().map_err(io::Error::other)?,
                    ));
                }
                output_index += 1;
            }
            adapter_index += 1;
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "original DXGI output unavailable",
    ))
}
pub(super) fn rotation(value: DXGI_MODE_ROTATION) -> io::Result<BgraRotation> {
    match value {
        DXGI_MODE_ROTATION_UNSPECIFIED | DXGI_MODE_ROTATION_IDENTITY => Ok(BgraRotation::Identity),
        DXGI_MODE_ROTATION_ROTATE90 => Ok(BgraRotation::Clockwise90),
        DXGI_MODE_ROTATION_ROTATE180 => Ok(BgraRotation::Clockwise180),
        DXGI_MODE_ROTATION_ROTATE270 => Ok(BgraRotation::Clockwise270),
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unknown DXGI desktop rotation",
        )),
    }
}
