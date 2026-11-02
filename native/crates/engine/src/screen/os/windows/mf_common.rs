//! Media Foundation for the screen strand (SCR-M3, decision C2), loaded at
//! run time: the functions of `mfplat.dll` come from `LoadLibraryExW`
//! (System32 only) and `GetProcAddress`, because a build that imported them
//! would not even start where Media Foundation is missing (Windows Server
//! without the feature, N editions). Media Foundation counts as present
//! when `MFStartup` succeeds and `MFTEnumEx` finds an H.264 encoder; nothing
//! is derived from the edition. The transforms are enumerated once per
//! process: hardware encoders (asynchronous, `mf_encoder`), the software
//! encoder (synchronous, `mf_encoder_sync`) and the decoder (`mf_decoder`);
//! each opens from its enumeration entry anew. Encoders take NV12 in system
//! memory (the GPU path follows with SCR-M4) and run in low-latency mode
//! without B frames, CBR at the session's target. Values an encoder refuses
//! stay at its default.
use std::cell::Cell;
use std::ffi::c_void;
use std::io;
use std::sync::{Arc, OnceLock};

use libloading::os::windows::{Library, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows::core::{GUID, HRESULT, PWSTR};
use windows::Win32::Media::MediaFoundation::{
    eAVEncH264VProfile_Base, eAVEncH264VProfile_ConstrainedBase, eAVEncH264VProfile_High,
    IMFActivate, IMFTransform, MFMediaType_Video, MFT_FRIENDLY_NAME_Attribute, MFVideoFormat_H264,
    MFVideoFormat_NV12, MFSTARTUP_NOSOCKET, MFT_CATEGORY_VIDEO_DECODER, MFT_CATEGORY_VIDEO_ENCODER,
    MFT_ENUM_FLAG, MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT,
    MFT_REGISTER_TYPE_INFO, MF_MT_FRAME_SIZE, MF_MT_MPEG2_LEVEL, MF_MT_MPEG2_PROFILE,
    MF_MT_SUBTYPE, MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};

use super::mf_decoder::MfDecoder;
use super::mf_encoder::AsyncEncoder;
use super::mf_encoder_sync::SyncEncoder;
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::codec_registry::{DecoderFactory, EncoderFactory};
use crate::screen::core::host_backend::{ScreenDecoder, ScreenEncoder};
use crate::screen::core::negotiate::{CodecChoice, CodecOffer, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire_binary::GopHeader;

type StartupFn = unsafe extern "system" fn(u32, u32) -> HRESULT;
type EnumFn = unsafe extern "system" fn(
    GUID,
    u32,
    *const MFT_REGISTER_TYPE_INFO,
    *const MFT_REGISTER_TYPE_INFO,
    *mut *mut Option<IMFActivate>,
    *mut u32,
) -> HRESULT;
type CreateFn = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type CreateBufferFn = unsafe extern "system" fn(u32, *mut *mut c_void) -> HRESULT;

/// The functions of `mfplat.dll`, loaded once and never unloaded.
pub(super) struct MediaFoundation {
    pub(super) enum_ex: EnumFn,
    pub(super) create_media_type: CreateFn,
    pub(super) create_sample: CreateFn,
    pub(super) create_memory_buffer: CreateBufferFn,
    _library: Library,
}

fn symbol<T: Copy>(library: &Library, name: &[u8]) -> io::Result<T> {
    // SAFETY: every caller names the documented signature (mfapi.h).
    let symbol = unsafe { library.get::<T>(name) }.map_err(io::Error::other)?;
    Ok(*symbol)
}

fn load() -> io::Result<MediaFoundation> {
    // SAFETY: only System32 is searched, so no planted DLL loads; the
    // initialisers of mfplat.dll are the system's own.
    let library = unsafe { Library::load_with_flags("mfplat.dll", LOAD_LIBRARY_SEARCH_SYSTEM32) }
        .map_err(|error| io::Error::new(io::ErrorKind::Unsupported, error))?;
    let startup: StartupFn = symbol(&library, b"MFStartup\0")?;
    let platform = MediaFoundation {
        enum_ex: symbol(&library, b"MFTEnumEx\0")?,
        create_media_type: symbol(&library, b"MFCreateMediaType\0")?,
        create_sample: symbol(&library, b"MFCreateSample\0")?,
        create_memory_buffer: symbol(&library, b"MFCreateMemoryBuffer\0")?,
        _library: library,
    };
    ensure_com();
    // SAFETY: MFStartup with the version of the headers; Media Foundation
    // stays started for the life of the process.
    unsafe { startup(MF_VERSION, MFSTARTUP_NOSOCKET) }
        .ok()
        .map_err(|error| io::Error::new(io::ErrorKind::Unsupported, error))?;
    Ok(platform)
}

/// Media Foundation of this process, or why it is missing.
pub(super) fn platform() -> io::Result<&'static MediaFoundation> {
    static PLATFORM: OnceLock<Result<MediaFoundation, String>> = OnceLock::new();
    PLATFORM
        .get_or_init(|| load().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|reason| io::Error::new(io::ErrorKind::Unsupported, reason.clone()))
}

/// Joins the multithreaded apartment once per thread; an apartment the
/// thread had before stays (the Media Foundation objects are free-threaded).
pub(super) fn ensure_com() {
    thread_local! {
        static READY: Cell<bool> = const { Cell::new(false) };
    }
    READY.with(|ready| {
        if !ready.get() {
            // SAFETY: no reserved pointer; the result only reports the mode.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            ready.set(true);
        }
    });
}

/// The three kinds of transform this strand uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    HardwareEncoder,
    SoftwareEncoder,
    Decoder,
}

impl Kind {
    fn query(self) -> (GUID, MFT_ENUM_FLAG, GUID, GUID) {
        let sorted = MFT_ENUM_FLAG_SORTANDFILTER;
        match self {
            Self::HardwareEncoder => (
                MFT_CATEGORY_VIDEO_ENCODER,
                MFT_ENUM_FLAG_HARDWARE | sorted,
                MFVideoFormat_NV12,
                MFVideoFormat_H264,
            ),
            Self::SoftwareEncoder => (
                MFT_CATEGORY_VIDEO_ENCODER,
                MFT_ENUM_FLAG_SYNCMFT | sorted,
                MFVideoFormat_NV12,
                MFVideoFormat_H264,
            ),
            Self::Decoder => (
                MFT_CATEGORY_VIDEO_DECODER,
                MFT_ENUM_FLAG_SYNCMFT | sorted,
                MFVideoFormat_H264,
                MFVideoFormat_NV12,
            ),
        }
    }
}

/// The registered transforms of `kind` in the order of their merit.
fn enumerate(kind: Kind) -> io::Result<Vec<IMFActivate>> {
    let platform = platform()?;
    ensure_com();
    let (category, flags, input, output) = kind.query();
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: input,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: output,
    };
    let mut array: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: both type infos outlive the call; the array is freed below.
    unsafe {
        (platform.enum_ex)(
            category,
            flags.0 as u32,
            &input,
            &output,
            &mut array,
            &mut count,
        )
    }
    .ok()
    .map_err(io::Error::other)?;
    if array.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: MFTEnumEx returned `count` entries at `array`; each reference
    // is taken out once, so it is released exactly once.
    let entries = unsafe { std::slice::from_raw_parts_mut(array, count as usize) };
    let found = entries.iter_mut().filter_map(Option::take).collect();
    // SAFETY: MFTEnumEx allocated the array with CoTaskMemAlloc.
    unsafe { CoTaskMemFree(Some(array.cast_const().cast())) };
    Ok(found)
}

fn friendly_name(activate: &IMFActivate) -> String {
    let mut value = PWSTR::null();
    let mut length = 0u32;
    // SAFETY: the string comes from CoTaskMemAlloc and is freed below.
    let read = unsafe {
        activate.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut value, &mut length)
    };
    if read.is_err() || value.is_null() {
        return "Media Foundation".into();
    }
    // SAFETY: `value` is a terminated wide string of `length` characters.
    let name = unsafe { value.to_string() }.unwrap_or_else(|_| "Media Foundation".into());
    // SAFETY: allocated by GetAllocatedString.
    unsafe { CoTaskMemFree(Some(value.0.cast_const().cast())) };
    name
}

/// The transform `index` of `kind`, activated, when it still is `name`.
pub(super) fn activate(kind: Kind, index: usize, name: &str) -> io::Result<IMFTransform> {
    let entries = enumerate(kind)?;
    let entry = entries
        .get(index)
        .filter(|entry| friendly_name(entry) == name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{name} is gone")))?;
    // SAFETY: activates a registered transform in the multithreaded
    // apartment of this thread.
    unsafe { entry.ActivateObject::<IMFTransform>() }.map_err(io::Error::other)
}

/// The names of the transforms found, per kind, once per process.
struct Found {
    hardware: Vec<String>,
    software: Vec<String>,
    decoders: Vec<String>,
}

fn names(kind: Kind) -> Vec<String> {
    enumerate(kind)
        .map(|entries| entries.iter().map(friendly_name).collect())
        .unwrap_or_default()
}

fn found() -> &'static Found {
    static FOUND: OnceLock<Found> = OnceLock::new();
    FOUND.get_or_init(|| Found {
        hardware: names(Kind::HardwareEncoder),
        software: names(Kind::SoftwareEncoder),
        decoders: names(Kind::Decoder),
    })
}

/// Media Foundation runs here and has an H.264 encoder (task tests).
#[cfg(test)]
pub(super) fn available() -> bool {
    let found = found();
    !found.hardware.is_empty() || !found.software.is_empty()
}

/// The factories of the transforms found: hardware encoders before the
/// software encoder, then the decoder.
pub(super) fn codecs() -> (Vec<Arc<dyn EncoderFactory>>, Vec<Arc<dyn DecoderFactory>>) {
    let found = found();
    let mut encoders: Vec<Arc<dyn EncoderFactory>> = Vec::new();
    for (kind, list) in [
        (Kind::HardwareEncoder, &found.hardware),
        (Kind::SoftwareEncoder, &found.software),
    ] {
        for (index, name) in list.iter().enumerate() {
            let name = name.clone();
            encoders.push(Arc::new(MfEncoderFactory {
                kind,
                index,
                name,
                caps: OnceLock::new(),
            }));
        }
    }
    let decoders: Vec<Arc<dyn DecoderFactory>> = found
        .decoders
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let name = name.clone();
            Arc::new(MfDecoderFactory {
                index,
                name,
                caps: OnceLock::new(),
            }) as Arc<dyn DecoderFactory>
        })
        .collect();
    (encoders, decoders)
}

/// What an encoder is opened for.
#[derive(Clone, Debug)]
pub(super) struct EncoderSetup {
    pub name: String,
    pub profile: H264Profile,
    pub size: FrameSize,
    pub fps: u32,
}

struct MfEncoderFactory {
    kind: Kind,
    index: usize,
    name: String,
    caps: OnceLock<Vec<CodecOffer>>,
}

impl EncoderFactory for MfEncoderFactory {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn offers(&self) -> Vec<CodecOffer> {
        let hardware = self.kind == Kind::HardwareEncoder;
        self.caps
            .get_or_init(|| {
                let mut offers = vec![
                    CodecOffer::h264(H264Profile::High, hardware),
                    CodecOffer::h264(H264Profile::ConstrainedBaseline, hardware),
                ];
                offers.extend(confirmed_offers(
                    self.kind, self.index, &self.name, hardware,
                ));
                offers
            })
            .clone()
    }

    fn open(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        self.create(choice, size, fps, None)
    }

    fn open_admitted(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: CaptureGrant,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        self.create(choice, size, fps, Some(grant))
    }
}

impl MfEncoderFactory {
    fn create(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: Option<CaptureGrant>,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        let profile = choice
            .profile
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "H.264 needs a profile"))?;
        crate::screen::core::frame_budget::FrameLimits::RESOURCE.check(size)?;
        if size.pixels() > choice.max_pixels {
            return Err(io::Error::other(
                "MF frame exceeds selected codec pixel limit",
            ));
        }
        if size != size.even() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Media Foundation encodes even sizes only",
            ));
        }
        ensure_com();
        let transform = activate(self.kind, self.index, &self.name)?;
        let setup = EncoderSetup {
            name: self.name.clone(),
            profile,
            size,
            fps: fps.max(1),
        };
        match (self.kind == Kind::HardwareEncoder, grant) {
            (true, Some(grant)) => Ok(Box::new(AsyncEncoder::new_admitted(
                transform, setup, grant,
            )?)),
            (false, Some(grant)) => Ok(Box::new(SyncEncoder::new_admitted(
                transform, setup, grant,
            )?)),
            (true, None) => Ok(Box::new(AsyncEncoder::new(transform, setup)?)),
            (false, None) => Ok(Box::new(SyncEncoder::new(transform, setup)?)),
        }
    }
}

struct MfDecoderFactory {
    index: usize,
    name: String,
    caps: OnceLock<Vec<CodecOffer>>,
}

impl DecoderFactory for MfDecoderFactory {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn offers(&self) -> Vec<CodecOffer> {
        self.caps
            .get_or_init(|| {
                let mut offers = vec![
                    CodecOffer::h264(H264Profile::High, false).with_max(FrameSize::MAX),
                    CodecOffer::h264(H264Profile::ConstrainedBaseline, false)
                        .with_max(FrameSize::MAX),
                ];
                offers.extend(confirmed_offers(
                    Kind::Decoder,
                    self.index,
                    &self.name,
                    false,
                ));
                offers
            })
            .clone()
    }

    fn open(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        ensure_com();
        let transform = activate(Kind::Decoder, self.index, &self.name)?;
        Ok(Box::new(MfDecoder::new(
            transform,
            self.name.clone(),
            header,
        )?))
    }
}

/// Only a complete advertised H.264 type is evidence for an extended dimension.
/// Partial/absent types leave the historical offer intact; SetType at open remains decisive.
fn confirmed_offers(kind: Kind, index: usize, name: &str, hardware: bool) -> Vec<CodecOffer> {
    let Ok(transform) = activate(kind, index, name) else {
        return Vec::new();
    };
    let mut offers = Vec::new();
    for index in 0..64 {
        // SAFETY: enumerating types on a newly activated transform, without streaming.
        let media = unsafe {
            if kind == Kind::Decoder {
                transform.GetInputAvailableType(0, index)
            } else {
                transform.GetOutputAvailableType(0, index)
            }
        };
        let Ok(media) = media else {
            break;
        };
        if unsafe { media.GetGUID(&MF_MT_SUBTYPE) }.ok() != Some(MFVideoFormat_H264) {
            continue;
        }
        let read = || -> windows::core::Result<(u32, u32, u64)> {
            unsafe {
                Ok((
                    media.GetUINT32(&MF_MT_MPEG2_PROFILE)?,
                    media.GetUINT32(&MF_MT_MPEG2_LEVEL)?,
                    media.GetUINT64(&MF_MT_FRAME_SIZE)?,
                ))
            }
        };
        let Ok((profile, level, packed)) = read() else {
            continue;
        };
        let profile = if profile == eAVEncH264VProfile_High.0 as u32 {
            H264Profile::High
        } else if profile == eAVEncH264VProfile_Base.0 as u32
            || profile == eAVEncH264VProfile_ConstrainedBase.0 as u32
        {
            H264Profile::ConstrainedBaseline
        } else {
            continue;
        };
        let Some(level) = crate::screen::core::frame_size_caps::avc_limits(level) else {
            continue;
        };
        let size = FrameSize::new((packed >> 32) as u32, packed as u32);
        if !level.supports(size, 0) {
            continue;
        }
        let max = crate::screen::core::frame_budget::FrameLimits::RESOURCE.max_size;
        let mut offer = CodecOffer::h264(profile, hardware).with_max(FrameSize::new(
            size.width.min(max.width),
            size.height.min(max.height),
        ));
        offer.max_macroblocks = Some(level.macroblocks);
        offer.max_macroblocks_per_second = Some(level.macroblocks_per_second);
        if !offers.contains(&offer) {
            offers.push(offer);
        }
    }
    offers
}
