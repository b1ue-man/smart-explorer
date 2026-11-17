//! The Direct3D 11 device of the Windows viewer (plan 2026-11-17 P8).
//! FFmpeg's D3D11VA stage decodes into textures of this device and the
//! viewer's presenter draws them into its swap chain on the same device, so
//! a picture never leaves the GPU and never crosses devices (Moonlight-qt
//! `app/streaming/video/ffmpeg-renderers/d3d11va.cpp:672-690` at 28629dc1:
//! decoder and renderer on one device, one lock). It is made on the default
//! hardware adapter with video and BGRA support and with multithread
//! protection, because FFmpeg (decode thread) and the presenter (decode
//! thread, or the window's thread for a deferred picture) use its immediate
//! context. The lock both take is the device's own recursive critical
//! section (`ID3D11Multithread::Enter`/`Leave`); FFmpeg gets it as
//! `AVD3D11VADeviceContext.lock`/`unlock`, which must be recursive
//! (`libavutil/hwcontext_d3d11va.h:82-96`). A lost device is replaced at the
//! next request.
use std::ffi::c_void;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use windows::core::Interface;
use windows::Win32::Foundation::{E_POINTER, HMODULE};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1,
    D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11VideoContext,
    ID3D11VideoDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
    D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0, D3D11_DECODER_PROFILE_H264_VLD_NOFGT,
    D3D11_DECODER_PROFILE_HEVC_VLD_MAIN, D3D11_SDK_VERSION, D3D11_VIDEO_DECODER_DESC,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_NV12;
use windows::Win32::Graphics::Dxgi::IDXGIAdapter;

use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;

/// The viewer's device with its immediate context and video interfaces.
pub struct ViewerDevice {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub video_device: ID3D11VideoDevice,
    pub video_context: ID3D11VideoContext,
    multithread: ID3D11Multithread,
}

// SAFETY: a D3D11 device is free-threaded; its immediate context (and the
// video context on it) is switched to multithread protection in `create`,
// and every sequence of calls that must not interleave takes `lock`.
unsafe impl Send for ViewerDevice {}
// SAFETY: as above.
unsafe impl Sync for ViewerDevice {}

/// The device's critical section, held until dropped.
pub struct DeviceLock<'a>(&'a ID3D11Multithread);

impl Drop for DeviceLock<'_> {
    fn drop(&mut self) {
        // SAFETY: `Enter` was called on the same interface in `lock`.
        unsafe { self.0.Leave() };
    }
}

fn create() -> io::Result<ViewerDevice> {
    let levels = [
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
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
    let multithread: ID3D11Multithread = context.cast()?;
    // SAFETY: a live interface; the previous setting is of no interest.
    let _ = unsafe { multithread.SetMultithreadProtected(true) };
    Ok(ViewerDevice {
        video_device: device.cast()?,
        video_context: context.cast()?,
        device,
        context,
        multithread,
    })
}

/// The viewer device of this process; a lost one is made anew.
pub fn shared() -> io::Result<Arc<ViewerDevice>> {
    static DEVICE: Mutex<Option<Arc<ViewerDevice>>> = Mutex::new(None);
    let mut slot = DEVICE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(device) = slot.as_ref().filter(|device| !device.lost()) {
        return Ok(device.clone());
    }
    *slot = None;
    let device = Arc::new(create()?);
    *slot = Some(device.clone());
    Ok(device)
}

/// The DXVA profile of `codec` as se streams it (8-bit 4:2:0), as
/// `windows` 0.58 binds them.
fn profile_of(codec: CodecKind) -> Option<windows::core::GUID> {
    match codec {
        CodecKind::H264 => Some(D3D11_DECODER_PROFILE_H264_VLD_NOFGT),
        CodecKind::Hevc => Some(D3D11_DECODER_PROFILE_HEVC_VLD_MAIN),
        CodecKind::Av1 => Some(D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0),
        CodecKind::Tiles => None,
    }
}

impl ViewerDevice {
    /// The device was removed, reset or hung.
    pub fn lost(&self) -> bool {
        // SAFETY: a plain query of the live device.
        unsafe { self.device.GetDeviceRemovedReason() }.is_err()
    }

    /// Holds the device's lock (FFmpeg takes the same one).
    pub fn lock(&self) -> DeviceLock<'_> {
        // SAFETY: `Leave` follows in `DeviceLock::drop`.
        unsafe { self.multithread.Enter() };
        DeviceLock(&self.multithread)
    }

    /// The lock for FFmpeg's callbacks: the raw `ID3D11Multithread`, valid
    /// while this device lives (FFmpeg's device context holds the device).
    pub(super) fn lock_context(&self) -> *mut c_void {
        self.multithread.as_raw()
    }

    /// Whether the device decodes `codec` into NV12, and a stream of `size`
    /// when given (a decoder configuration for the aligned size).
    pub fn decodes(&self, codec: CodecKind, size: Option<FrameSize>) -> bool {
        let Some(profile) = profile_of(codec) else {
            return false;
        };
        // SAFETY: plain queries of the video device.
        let format = unsafe {
            self.video_device
                .CheckVideoDecoderFormat(&profile, DXGI_FORMAT_NV12)
        };
        if !format.is_ok_and(|supported| supported.as_bool()) {
            return false;
        }
        let Some(size) = size else {
            return true;
        };
        let align = |value: u32| value.checked_add(15).map(|value| value & !15);
        let (Some(width), Some(height)) = (align(size.width), align(size.height)) else {
            return false;
        };
        let desc = D3D11_VIDEO_DECODER_DESC {
            Guid: profile,
            SampleWidth: width,
            SampleHeight: height,
            OutputFormat: DXGI_FORMAT_NV12,
        };
        // SAFETY: `desc` lives through the call.
        unsafe { self.video_device.GetVideoDecoderConfigCount(&desc) }.is_ok_and(|count| count > 0)
    }
}

/// `AVD3D11VADeviceContext.lock`: enters the device's critical section.
pub(super) extern "C" fn lock_callback(context: *mut c_void) {
    // SAFETY: FFmpeg passes back `lock_context`, a live ID3D11Multithread.
    unsafe {
        if let Some(multithread) = ID3D11Multithread::from_raw_borrowed(&context) {
            multithread.Enter();
        }
    }
}

/// `AVD3D11VADeviceContext.unlock`.
pub(super) extern "C" fn unlock_callback(context: *mut c_void) {
    // SAFETY: as in `lock_callback`; FFmpeg pairs every unlock with a lock.
    unsafe {
        if let Some(multithread) = ID3D11Multithread::from_raw_borrowed(&context) {
            multithread.Leave();
        }
    }
}
