//! Media Foundation side of the texture input (plan 2026-11-14 bildkette
//! B-20, package BK2a; moved out of `mf_d3d_input.rs` by BK12 so that file
//! stays below 500 lines): the device manager handed to a hardware MFT and
//! the DXGI surface buffer around an input texture. `MFCreateDXGIDeviceManager`
//! and `MFCreateDXGISurfaceBuffer` are loaded from System32 like
//! `mf_common::load` (an import would stop the program from starting where
//! Media Foundation is missing).
use std::ffi::c_void;
use std::io;
use std::sync::OnceLock;

use libloading::os::windows::{Library, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows::core::{Interface, GUID, HRESULT};
use windows::Win32::Foundation::BOOL;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer, IMFDXGIDeviceManager, IMFMediaBuffer, IMFSample, IMFTransform,
    MFT_MESSAGE_SET_D3D_MANAGER,
};

use crate::screen::os::mf_capture_media::sample_with_buffer;

type CreateManagerFn = unsafe extern "system" fn(*mut u32, *mut *mut c_void) -> HRESULT;
type SurfaceBufferFn =
    unsafe extern "system" fn(*const GUID, *mut c_void, u32, BOOL, *mut *mut c_void) -> HRESULT;

struct Functions {
    manager: CreateManagerFn,
    surface: SurfaceBufferFn,
    _library: Library,
}

fn functions() -> io::Result<&'static Functions> {
    static FUNCTIONS: OnceLock<Result<Functions, String>> = OnceLock::new();
    FUNCTIONS
        .get_or_init(|| {
            // SAFETY: System32 only; both symbols have the signatures of mfapi.h.
            unsafe {
                let library = Library::load_with_flags("mfplat.dll", LOAD_LIBRARY_SEARCH_SYSTEM32)
                    .map_err(|error| error.to_string())?;
                let manager = *library
                    .get::<CreateManagerFn>(b"MFCreateDXGIDeviceManager\0")
                    .map_err(|error| error.to_string())?;
                let surface = *library
                    .get::<SurfaceBufferFn>(b"MFCreateDXGISurfaceBuffer\0")
                    .map_err(|error| error.to_string())?;
                Ok(Functions {
                    manager,
                    surface,
                    _library: library,
                })
            }
        })
        .as_ref()
        .map_err(|reason| io::Error::new(io::ErrorKind::Unsupported, reason.clone()))
}

/// A DXGI device manager of `device`, handed to `transform` before its
/// media types are set.
pub(super) fn hand_manager(
    transform: &IMFTransform,
    device: &ID3D11Device,
) -> io::Result<IMFDXGIDeviceManager> {
    let mut token = 0u32;
    let mut pointer = std::ptr::null_mut();
    // SAFETY: writes a token and one new reference.
    unsafe { (functions()?.manager)(&mut token, &mut pointer) }.ok()?;
    if pointer.is_null() {
        return Err(io::Error::other("no DXGI device manager"));
    }
    // SAFETY: the reference written above is owned from here on.
    let manager = unsafe { IMFDXGIDeviceManager::from_raw(pointer) };
    // SAFETY: binds the live capture device with the manager's token; the
    // transform takes its own reference to the manager.
    unsafe {
        manager.ResetDevice(device, token)?;
        transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)?;
    }
    Ok(manager)
}

/// Takes the device manager back from `transform` (system memory input).
pub(in crate::screen::os) fn detach(transform: &IMFTransform) {
    // SAFETY: a zero manager clears the transform's device manager.
    let _ = unsafe { transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, 0) };
}

/// A sample whose buffer is `texture` (subresource 0).
pub(super) fn surface_sample(
    texture: &ID3D11Texture2D,
    time: i64,
    duration: i64,
) -> io::Result<IMFSample> {
    let mut raw = std::ptr::null_mut();
    // SAFETY: a live texture; MF writes one new buffer reference.
    unsafe {
        (functions()?.surface)(
            &ID3D11Texture2D::IID,
            texture.as_raw(),
            0,
            BOOL(0),
            &mut raw,
        )
    }
    .ok()?;
    // SAFETY: the reference written above is owned from here on.
    let buffer = unsafe { IMFMediaBuffer::from_raw(raw) };
    let two = buffer.cast::<IMF2DBuffer>();
    // SAFETY: the buffer's own surface length.
    if let Ok(length) = two.and_then(|two| unsafe { two.GetContiguousLength() }) {
        // SAFETY: plain attribute write on the owned buffer.
        unsafe { buffer.SetCurrentLength(length)? };
    }
    let sample = sample_with_buffer(&buffer)?;
    // SAFETY: attribute writes on the owned sample.
    unsafe {
        sample.SetSampleTime(time)?;
        sample.SetSampleDuration(duration)?;
    }
    Ok(sample)
}
