//! Capture of a monitor with the DXGI desktop duplication (SCR-M4, U16-7):
//! the fallback where Graphics Capture is not available (before Windows 10
//! 1903) or does not start for a monitor. Windows can not be captured this
//! way, and the pointer is not part of the picture (the limit
//! `no_cursor_capture`). The device is created on the adapter that owns the
//! output, as a duplication requires.
use std::io;
use std::time::Duration;

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{ID3D11Texture2D, D3D11_TEXTURE2D_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
    IDXGIResource, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};

use super::capture_source::{Grabber, FIRST_FRAME_WAIT};
use super::d3d_device::{self, delivered_size, DeviceSet, Readback};
use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::session_state::SessionState;

/// The duplication of an output and the device it was made with.
struct Duplication {
    devices: DeviceSet,
    duplication: IDXGIOutputDuplication,
    size: FrameSize,
}

/// The adapter and the output interface of the GDI output `device`
/// (`\\.\DISPLAYn`).
fn find_output(device: &str) -> windows::core::Result<(IDXGIAdapter, IDXGIOutput1)> {
    // SAFETY: plain enumeration through DXGI; every interface is owned.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut adapter_index = 0;
        while let Ok(adapter) = factory.EnumAdapters1(adapter_index) {
            let mut output_index = 0;
            while let Ok(output) = adapter.EnumOutputs(output_index) {
                let described = output.GetDesc()?;
                let end = described
                    .DeviceName
                    .iter()
                    .position(|unit| *unit == 0)
                    .unwrap_or(described.DeviceName.len());
                if String::from_utf16_lossy(&described.DeviceName[..end]) == device {
                    let adapter: IDXGIAdapter = adapter.cast()?;
                    return Ok((adapter, output.cast()?));
                }
                output_index += 1;
            }
            adapter_index += 1;
        }
    }
    Err(windows::core::Error::from(
        windows::Win32::Foundation::E_FAIL,
    ))
}

fn duplicate(device: &str) -> windows::core::Result<Duplication> {
    let (adapter, output) = find_output(device)?;
    let devices = d3d_device::create_on(Some(&adapter))?;
    // SAFETY: the output and the device belong to the same adapter.
    let duplication = unsafe { output.DuplicateOutput(&devices.device)? };
    // SAFETY: a plain query of the duplication just made.
    let description = unsafe { duplication.GetDesc() };
    let size = delivered_size(description.ModeDesc.Width, description.ModeDesc.Height);
    Ok(Duplication {
        devices,
        duplication,
        size,
    })
}

/// One monitor recorded with the desktop duplication.
pub(super) struct DxgiCapture {
    device: String,
    live: Duplication,
    readback: Readback,
    last: Option<BgraImage>,
}

impl DxgiCapture {
    /// Duplicates the monitor whose GDI output is `device` (`\\.\DISPLAYn`).
    pub(super) fn open(device: &str) -> windows::core::Result<Self> {
        Ok(Self {
            device: device.into(),
            live: duplicate(device)?,
            readback: Readback::default(),
            last: None,
        })
    }

    /// The last picture again when the caller needs one, else "unchanged".
    fn again(&self, keyframe: bool) -> SourceFrame {
        match (&self.last, keyframe) {
            (Some(last), true) => SourceFrame::Frame(last.clone()),
            (Some(_), false) => SourceFrame::Unchanged,
            (None, _) => SourceFrame::Paused(SessionState::Starting),
        }
    }

    fn read(&mut self, resource: &IDXGIResource) -> io::Result<BgraImage> {
        let texture: ID3D11Texture2D = resource.cast().map_err(io::Error::other)?;
        let mut description = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: a valid out structure.
        unsafe { texture.GetDesc(&mut description) };
        self.readback.read(
            &self.live.devices,
            &texture,
            description.Width,
            description.Height,
        )
    }
}

impl Grabber for DxgiCapture {
    fn size(&self) -> FrameSize {
        self.live.size
    }

    fn capture(&mut self, keyframe: bool) -> io::Result<SourceFrame> {
        // The first picture may take a display refresh; later calls only
        // look whether one is ready.
        let wait = if self.last.is_none() {
            FIRST_FRAME_WAIT
        } else {
            Duration::ZERO
        };
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        // SAFETY: the out pointers address locals; a frame acquired here is
        // released below before the next call.
        let acquired = unsafe {
            self.live.duplication.AcquireNextFrame(
                wait.as_millis() as u32,
                &mut info,
                &mut resource,
            )
        };
        match acquired {
            Ok(()) => {
                // Only the pointer moved: the desktop is as it was.
                let picture = match resource {
                    Some(resource) if info.AccumulatedFrames > 0 || self.last.is_none() => {
                        Some(self.read(&resource))
                    }
                    _ => None,
                };
                // SAFETY: the frame was acquired above.
                let _ = unsafe { self.live.duplication.ReleaseFrame() };
                match picture {
                    Some(image) => {
                        let image = image?;
                        self.live.size = image.size();
                        self.last = Some(image.clone());
                        Ok(SourceFrame::Frame(image))
                    }
                    None => Ok(self.again(keyframe)),
                }
            }
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => Ok(self.again(keyframe)),
            Err(error) if error.code() == DXGI_ERROR_ACCESS_LOST => {
                self.reopen()?;
                Ok(SourceFrame::Paused(SessionState::Starting))
            }
            Err(error) => Err(io::Error::other(error)),
        }
    }

    fn reopen(&mut self) -> io::Result<()> {
        self.live = duplicate(&self.device).map_err(io::Error::other)?;
        self.readback.reset();
        self.last = None;
        Ok(())
    }
}
