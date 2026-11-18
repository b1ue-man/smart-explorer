//! Direct3D 11 for the viewer's Media Foundation H.264 decoder (plan
//! `bildkette.md` BK5w, B-37 part 1): one video-capable device of the
//! default adapter per process, its DXGI device manager for
//! `MFT_MESSAGE_SET_D3D_MANAGER` (the decoder then decodes with DXVA,
//! D3D11VA, into NV12 textures), the question whether the device decodes an
//! H.264 stream of a given size, and the read-back of a decoded texture into
//! NV12 planes in system memory.
//!
//! Hand-over point for a shared texture into wgpu without read-back:
//! `output_texture` returns the decoder's NV12 texture and its subresource
//! index (decoders output into a texture array) on `DecodeDevice::device`.
//! BK9 found no import into the viewer window's device with the locked
//! wgpu 22 (reasons and the enabling change in `G/screen_present.rs`), so
//! every picture goes through one staging copy, read straight into the
//! output planes of the plane path (`YuvPicture`, `G/screen_wgpu_yuv.rs`).
//!
//! `MFCreateDXGIDeviceManager` comes from `mfplat.dll` at run time like the
//! other Media Foundation functions (`mf_common`): an import would stop the
//! program from starting where Media Foundation is missing.
use std::ffi::c_void;
use std::io;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use libloading::os::windows::{Library, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows::core::{IUnknown, Interface, HRESULT};
use windows::Win32::Foundation::{E_POINTER, HMODULE};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1,
    D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_9_3,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
    ID3D11VideoDevice, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0,
    D3D11_DECODER_PROFILE_H264_VLD_NOFGT, D3D11_DECODER_PROFILE_HEVC_VLD_MAIN,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING, D3D11_VIDEO_DECODER_DESC,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter, DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
    DXGI_ERROR_DRIVER_INTERNAL_ERROR,
};
use windows::Win32::Media::MediaFoundation::{
    IMFDXGIBuffer, IMFDXGIDeviceManager, IMFSample, IMFTransform, MFT_MESSAGE_SET_D3D_MANAGER,
    MF_E_UNSUPPORTED_D3D_TYPE, MF_E_UNSUPPORTED_MEDIATYPE_AT_D3D_FEATURE_LEVEL, MF_SA_D3D11_AWARE,
};

use super::stage;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;

// The decisions of `mf_decoder_stage` name these codes without Windows.
const _: () = assert!(DXGI_ERROR_DEVICE_REMOVED.0 == stage::DXGI_DEVICE_REMOVED);
const _: () = assert!(DXGI_ERROR_DEVICE_HUNG.0 == stage::DXGI_DEVICE_HUNG);
const _: () = assert!(DXGI_ERROR_DEVICE_RESET.0 == stage::DXGI_DEVICE_RESET);
const _: () = assert!(DXGI_ERROR_DRIVER_INTERNAL_ERROR.0 == stage::DXGI_DRIVER_INTERNAL_ERROR);
const _: () = assert!(MF_E_UNSUPPORTED_D3D_TYPE.0 == stage::MF_UNSUPPORTED_D3D_TYPE);
const _: () = assert!(
    MF_E_UNSUPPORTED_MEDIATYPE_AT_D3D_FEATURE_LEVEL.0 == stage::MF_UNSUPPORTED_AT_FEATURE_LEVEL
);

/// The HRESULT an error carries (`?` keeps it as the raw OS error).
pub(super) fn hresult_of(error: &io::Error) -> Option<i32> {
    error.raw_os_error().or_else(|| {
        error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<windows::core::Error>())
            .map(|inner| inner.code().0)
    })
}

type CreateManagerFn = unsafe extern "system" fn(*mut u32, *mut *mut c_void) -> HRESULT;

/// `MFCreateDXGIDeviceManager` of `mfplat.dll`, loaded once.
fn create_manager_fn() -> io::Result<CreateManagerFn> {
    static FUNCTION: OnceLock<Result<(CreateManagerFn, Library), String>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            // SAFETY: System32 only, as in `mf_common::load`; the symbol has
            // the signature of mfapi.h.
            unsafe {
                let library = Library::load_with_flags("mfplat.dll", LOAD_LIBRARY_SEARCH_SYSTEM32)
                    .map_err(|error| error.to_string())?;
                let function = *library
                    .get::<CreateManagerFn>(b"MFCreateDXGIDeviceManager\0")
                    .map_err(|error| error.to_string())?;
                Ok((function, library))
            }
        })
        .as_ref()
        .map(|(function, _)| *function)
        .map_err(|reason| io::Error::new(io::ErrorKind::Unsupported, reason.clone()))
}

/// The decoding device of this process and its device manager.
pub(super) struct DecodeDevice {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    video: ID3D11VideoDevice,
    manager: IMFDXGIDeviceManager,
}

// SAFETY: a D3D11 device is free-threaded, its immediate context is switched
// to multithread protection in `create`, and the DXGI device manager
// serialises its users itself (LockDevice). Decoders of one process use the
// device one after another or under that protection.
unsafe impl Send for DecodeDevice {}
// SAFETY: as above.
unsafe impl Sync for DecodeDevice {}

fn create() -> io::Result<DecodeDevice> {
    super::super::mf_common::platform()?;
    let levels = [
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
        D3D_FEATURE_LEVEL_9_3,
    ];
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    // SAFETY: the out pointers address locals that live through the call.
    // Hardware only: WARP has no video decoder.
    unsafe {
        D3D11CreateDevice(
            None::<&IDXGIAdapter>,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE(std::ptr::null_mut()),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    let (Some(device), Some(context)) = (device, context) else {
        return Err(windows::core::Error::from(E_POINTER).into());
    };
    // The decoder uses the device from Media Foundation's calls while the
    // read-back uses the immediate context.
    if let Ok(protect) = context.cast::<ID3D11Multithread>() {
        // SAFETY: a live interface; the previous setting is of no interest.
        let _ = unsafe { protect.SetMultithreadProtected(true) };
    }
    let video: ID3D11VideoDevice = device.cast()?;
    let create_manager = create_manager_fn()?;
    let mut token = 0u32;
    let mut raw = std::ptr::null_mut();
    // SAFETY: MFCreateDXGIDeviceManager writes a token and one new reference.
    unsafe { create_manager(&mut token, &mut raw) }.ok()?;
    if raw.is_null() {
        return Err(windows::core::Error::from(E_POINTER).into());
    }
    // SAFETY: the reference written above is owned from here on.
    let manager = unsafe { IMFDXGIDeviceManager::from_raw(raw) };
    let unknown: IUnknown = device.cast()?;
    // SAFETY: binds the live device to the manager with its own token.
    unsafe { manager.ResetDevice(&unknown, token)? };
    Ok(DecodeDevice {
        device,
        context,
        video,
        manager,
    })
}

/// The device of this process; a lost device is replaced by a new one.
pub(super) fn shared() -> io::Result<Arc<DecodeDevice>> {
    static DEVICE: Mutex<Option<Arc<DecodeDevice>>> = Mutex::new(None);
    let mut slot = DEVICE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(device) = slot.as_ref().filter(|device| !device.lost()) {
        return Ok(device.clone());
    }
    *slot = None;
    let device = Arc::new(create()?);
    *slot = Some(device.clone());
    Ok(device)
}

/// The DXVA profile of `codec` as se streams it (8-bit 4:2:0): H.264 VLD
/// without film grain (Constrained Baseline, Main, High), HEVC Main (BK6),
/// AV1 profile 0 = Main (BK6); GUIDs as `windows` 0.58 binds them.
fn profile_of(codec: CodecKind) -> Option<windows::core::GUID> {
    match codec {
        CodecKind::H264 => Some(D3D11_DECODER_PROFILE_H264_VLD_NOFGT),
        CodecKind::Hevc => Some(D3D11_DECODER_PROFILE_HEVC_VLD_MAIN),
        CodecKind::Av1 => Some(D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0),
        CodecKind::Tiles => None,
    }
}

/// Whether this host decodes `codec` in hardware at all (`se screen codec`).
/// The answer is kept once the device was made; without it the next call
/// asks again (fix of 2026-11-18: one failure left the process without
/// hardware offers).
pub(super) fn hardware_offered(codec: CodecKind) -> bool {
    static OFFERED: Mutex<Option<[bool; 3]>> = Mutex::new(None);
    let mut slot = OFFERED.lock().unwrap_or_else(PoisonError::into_inner);
    let offered = match *slot {
        Some(offered) => offered,
        None => {
            let Ok(device) = shared() else {
                return false;
            };
            let offered = [CodecKind::H264, CodecKind::Hevc, CodecKind::Av1]
                .map(|codec| device.decodes(codec));
            *slot = Some(offered);
            offered
        }
    };
    match codec {
        CodecKind::H264 => offered[0],
        CodecKind::Hevc => offered[1],
        CodecKind::Av1 => offered[2],
        CodecKind::Tiles => false,
    }
}

impl DecodeDevice {
    /// The device was removed, reset or hung.
    pub(super) fn lost(&self) -> bool {
        // SAFETY: a plain query of the live device.
        unsafe { self.device.GetDeviceRemovedReason() }.is_err()
    }

    /// `codec` in the profile of `profile_of` into NV12.
    fn decodes(&self, codec: CodecKind) -> bool {
        let Some(profile) = profile_of(codec) else {
            return false;
        };
        // SAFETY: plain queries of the video device.
        unsafe {
            self.video
                .CheckVideoDecoderFormat(&profile, DXGI_FORMAT_NV12)
                .is_ok_and(|supported| supported.as_bool())
        }
    }

    /// Whether the device has a decoder configuration of `codec` for a
    /// stream of `size` (both H.264 profiles of se use the same VLD profile).
    pub(super) fn fits(&self, codec: CodecKind, size: FrameSize) -> bool {
        let align = |value: u32| value.checked_add(15).map(|value| value & !15);
        let (Some(width), Some(height), Some(profile)) =
            (align(size.width), align(size.height), profile_of(codec))
        else {
            return false;
        };
        let desc = D3D11_VIDEO_DECODER_DESC {
            Guid: profile,
            SampleWidth: width,
            SampleHeight: height,
            OutputFormat: DXGI_FORMAT_NV12,
        };
        // SAFETY: `desc` lives through the call.
        self.decodes(codec)
            && unsafe { self.video.GetVideoDecoderConfigCount(&desc) }.is_ok_and(|count| count > 0)
    }

    /// Hands the device manager to `transform` (before its media types are
    /// set); a transform that is not D3D11-aware refuses.
    pub(super) fn attach(&self, transform: &IMFTransform) -> io::Result<()> {
        // SAFETY: attribute read and one message on the activated transform;
        // the transform takes its own reference to the manager.
        unsafe {
            let aware = transform
                .GetAttributes()
                .and_then(|attributes| attributes.GetUINT32(&MF_SA_D3D11_AWARE))
                .unwrap_or(0);
            if aware == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "the Media Foundation decoder is not D3D11-aware",
                ));
            }
            transform
                .ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, self.manager.as_raw() as usize)?;
        }
        Ok(())
    }
}

/// The decoder's NV12 texture of an output sample and its subresource
/// index: the hand-over point for BK9.
pub(super) fn output_texture(sample: &IMFSample) -> io::Result<(ID3D11Texture2D, u32)> {
    // SAFETY: queries on the output sample owned by the caller; the texture
    // reference written by GetResource is owned from here on.
    unsafe {
        let buffer: IMFDXGIBuffer = sample.GetBufferByIndex(0)?.cast()?;
        let mut raw = std::ptr::null_mut();
        buffer.GetResource(&ID3D11Texture2D::IID, &mut raw)?;
        if raw.is_null() {
            return Err(windows::core::Error::from(E_POINTER).into());
        }
        let texture = ID3D11Texture2D::from_raw(raw);
        let index = buffer.GetSubresourceIndex()?;
        Ok((texture, index))
    }
}

/// Whether the output sample holds a decoder texture (a DXGI buffer) and
/// not a buffer in system memory.
pub(super) fn is_texture(sample: &IMFSample) -> bool {
    // SAFETY: queries on the output sample owned by the caller.
    unsafe {
        sample
            .GetBufferByIndex(0)
            .is_ok_and(|buffer| buffer.cast::<IMFDXGIBuffer>().is_ok())
    }
}

/// The read-back of decoded textures through one staging texture.
pub(super) struct TextureReadback {
    device: Arc<DecodeDevice>,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
}

// SAFETY: see `DecodeDevice`; the staging texture belongs to that device.
unsafe impl Send for TextureReadback {}

impl TextureReadback {
    pub(super) fn new(device: Arc<DecodeDevice>) -> Self {
        Self {
            device,
            staging: None,
        }
    }

    pub(super) fn device_lost(&self) -> bool {
        self.device.lost()
    }

    fn staging(&mut self, width: u32, height: u32) -> io::Result<ID3D11Texture2D> {
        if let Some((texture, w, h)) = &self.staging {
            if (*w, *h) == (width, height) {
                return Ok(texture.clone());
            }
        }
        self.staging = None;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut texture = None;
        // SAFETY: a checked description and a live out pointer.
        unsafe {
            self.device
                .device
                .CreateTexture2D(&desc, None, Some(&mut texture))?;
        }
        let texture = texture.ok_or_else(|| io::Error::other("no NV12 staging texture"))?;
        self.staging = Some((texture.clone(), width, height));
        Ok(texture)
    }

    /// Reads the picture of `sample` (its `picture` area) into `luma` and
    /// `chroma` with a common row pitch, which it returns; at most `max`
    /// bytes in all.
    pub(super) fn read(
        &mut self,
        sample: &IMFSample,
        picture: FrameSize,
        max: usize,
        luma: &mut Vec<u8>,
        chroma: &mut Vec<u8>,
    ) -> io::Result<usize> {
        let (texture, index) = output_texture(sample)?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: a live texture and an out structure.
        unsafe { texture.GetDesc(&mut desc) };
        if desc.Format != DXGI_FORMAT_NV12
            || desc.SampleDesc.Count != 1
            || desc.Width < picture.width
            || desc.Height < picture.height
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "decoded texture is not NV12 of the stream's size",
            ));
        }
        let staging = self.staging(desc.Width, desc.Height)?;
        let context = &self.device.context;
        // SAFETY: same size and format; the whole subresource is copied.
        unsafe { context.CopySubresourceRegion(&staging, 0, 0, 0, 0, &texture, index, None) };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: the staging texture has CPU read access.
        unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))? };
        let pitch = mapped.RowPitch as usize;
        let rows = picture.height as usize;
        let chroma_rows = rows.div_ceil(2);
        let plane = pitch.checked_mul(desc.Height as usize);
        let wanted = pitch
            .checked_mul(rows + chroma_rows)
            .filter(|bytes| *bytes <= max);
        let result = match (plane, wanted) {
            (Some(plane), Some(_))
                if pitch >= picture.width as usize && !mapped.pData.is_null() =>
            {
                // SAFETY: Map supplies `Height` luma rows and `Height / 2`
                // chroma rows of `RowPitch` bytes; only the picture's rows of
                // both planes are read.
                unsafe {
                    let base = mapped.pData as *const u8;
                    luma.clear();
                    luma.extend_from_slice(std::slice::from_raw_parts(base, pitch * rows));
                    chroma.clear();
                    chroma.extend_from_slice(std::slice::from_raw_parts(
                        base.add(plane),
                        pitch * chroma_rows,
                    ));
                }
                Ok(pitch)
            }
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "mapped NV12 texture has an unusable pitch",
            )),
        };
        // SAFETY: Map succeeded above.
        unsafe { context.Unmap(&staging, 0) };
        result
    }
}

#[cfg(test)]
#[path = "mf_decoder_d3d_tests.rs"]
mod tests;
