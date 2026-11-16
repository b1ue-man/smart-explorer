//! Media Foundation for the screen strand (SCR-M3, decision C2), loaded at
//! run time: the functions of `mfplat.dll` come from `LoadLibraryExW`
//! (System32 only) and `GetProcAddress`, because a build that imported them
//! would not even start where Media Foundation is missing (Windows Server
//! without the feature, N editions). Media Foundation counts as present
//! when `MFStartup` succeeds and `MFTEnumEx` finds an H.264 encoder; nothing
//! is derived from the edition. The transforms are enumerated once per
//! process (`mf_enum.rs`): hardware encoders (asynchronous, `mf_encoder`),
//! the software encoder (synchronous, `mf_encoder_sync`) and the decoders
//! (`mf_decoder`); each opens from its enumeration entry anew. Hardware
//! encoders take textures of the capture device where they can
//! (`mf_d3d_input.rs`, BK2a), else NV12 in system memory, and run in
//! low-latency mode without B frames, CBR at the session's target. Values
//! an encoder refuses stay at its default and are listed in `se screen
//! codec --json` (`refused_settings`, a probe of the MFT, merge W1).
//!
//! HEVC and AV1 (plan 2026-11-14 „Bildkette“ BK6, B-23): the decoders of
//! both are registered wherever an MFT decodes them; hardware encoders of
//! both are enumerated and registered with the codec in `EncoderSetup`,
//! whose output subtype and profile `mf_encoder_sync::output_type` writes (merge
//! W1, `MF_VIDEO_ENCODERS_CONNECTED`).
//!
//! Hybrid laptops (merge W1, plan 4.10a): hardware encoders carry their
//! adapter (`mf_enum::Entry`); with an admitted capture, an MFT of another
//! adapter gives way while the capture's adapter has an MFT of the codec.
use std::cell::Cell;
use std::ffi::c_void;
use std::io;
use std::sync::{Arc, OnceLock};

use libloading::os::windows::{Library, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows::core::{GUID, HRESULT};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, MFSTARTUP_NOSOCKET, MFT_REGISTER_TYPE_INFO, MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use super::mf_decoder::MfDecoder;
use super::mf_encoder::AsyncEncoder;
use super::mf_encoder_sync::SyncEncoder;
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::codec_registry::{DecoderFactory, EncoderFactory};
use crate::screen::core::host_backend::{ScreenDecoder, ScreenEncoder};
use crate::screen::core::negotiate::{CodecChoice, CodecKind, CodecOffer, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire_binary::GopHeader;

#[path = "mf_enum.rs"]
mod mf_enum;
#[cfg(any(test, feature = "test-support"))]
pub(super) use mf_enum::available;
use mf_enum::confirmed_offers;
pub(super) use mf_enum::{
    activate_codec, adapter_of, prefer_capture_adapter, subtype, Entry, Kind,
};

/// Whether the encoders write HEVC and AV1 output types (B-23): since merge
/// W1 `mf_encoder_sync::output_type` chooses the output subtype and profile by
/// `EncoderSetup::codec`, so HEVC and AV1 hardware MFTs offer their codec.
pub(super) const MF_VIDEO_ENCODERS_CONNECTED: bool = true;

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
/// `MFTEnum2` (Windows 10 1703 and later) and `MFCreateAttributes`, as the
/// crate `windows` 0.58.0 binds them (`link!` lines of
/// `Win32::Media::MediaFoundation`).
type Enum2Fn = unsafe extern "system" fn(
    GUID,
    u32,
    *const MFT_REGISTER_TYPE_INFO,
    *const MFT_REGISTER_TYPE_INFO,
    *mut c_void,
    *mut *mut Option<IMFActivate>,
    *mut u32,
) -> HRESULT;
type CreateAttributesFn = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;

/// The functions of `mfplat.dll`, loaded once and never unloaded.
pub(super) struct MediaFoundation {
    pub(super) enum_ex: EnumFn,
    /// Per-adapter enumeration (merge W1); `None` before Windows 10 1703.
    pub(super) enum2: Option<Enum2Fn>,
    pub(super) create_attributes: Option<CreateAttributesFn>,
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
        enum2: symbol(&library, b"MFTEnum2\0").ok(),
        create_attributes: symbol(&library, b"MFCreateAttributes\0").ok(),
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

/// The factories of the transforms found: hardware encoders before the
/// software encoder, then the decoders; HEVC and AV1 after H.264 (BK6).
pub(super) fn codecs() -> (Vec<Arc<dyn EncoderFactory>>, Vec<Arc<dyn DecoderFactory>>) {
    let found = mf_enum::found();
    let mut encoders: Vec<Arc<dyn EncoderFactory>> = Vec::new();
    let software: Vec<Entry> = found
        .software
        .iter()
        .map(|name| Entry {
            name: name.clone(),
            adapter: None,
        })
        .collect();
    let h264 = [
        (Kind::HardwareEncoder, CodecKind::H264, &found.hardware),
        (Kind::SoftwareEncoder, CodecKind::H264, &software),
    ];
    let newer = found
        .video_encoders
        .iter()
        .map(|(codec, list)| (Kind::HardwareEncoder, *codec, list));
    for (kind, codec, list) in h264.into_iter().chain(newer) {
        for (index, entry) in list.iter().enumerate() {
            encoders.push(Arc::new(MfEncoderFactory {
                kind,
                codec,
                index,
                name: entry.name.clone(),
                adapter: entry.adapter,
                caps: OnceLock::new(),
                refused: OnceLock::new(),
            }));
        }
    }
    let mut decoders: Vec<Arc<dyn DecoderFactory>> = Vec::new();
    let all = std::iter::once((CodecKind::H264, &found.decoders)).chain(
        found
            .video_decoders
            .iter()
            .map(|(codec, list)| (*codec, list)),
    );
    for (codec, list) in all {
        for (index, name) in list.iter().enumerate() {
            decoders.push(Arc::new(MfDecoderFactory {
                codec,
                index,
                name: name.clone(),
                caps: OnceLock::new(),
            }));
        }
    }
    (encoders, decoders)
}

/// What an encoder is opened for.
#[derive(Clone, Debug)]
pub(super) struct EncoderSetup {
    pub name: String,
    pub profile: H264Profile,
    pub size: FrameSize,
    pub fps: u32,
    /// H.264, or HEVC/AV1 once `MF_VIDEO_ENCODERS_CONNECTED` (BK6, B-23).
    pub codec: CodecKind,
    /// The adapter of a hardware MFT (packed LUID, merge W1), `None` when
    /// unknown.
    pub adapter: Option<u64>,
}

struct MfEncoderFactory {
    kind: Kind,
    codec: CodecKind,
    index: usize,
    name: String,
    adapter: Option<u64>,
    caps: OnceLock<Vec<CodecOffer>>,
    refused: OnceLock<Vec<String>>,
}

impl EncoderFactory for MfEncoderFactory {
    fn name(&self) -> String {
        self.name.clone()
    }

    /// B-21 on a probe of the hardware MFT (`mf_encoder_sync::probe_refused`): the
    /// settings it leaves at its default, once per process.
    fn refused_settings(&self) -> Vec<String> {
        if self.kind != Kind::HardwareEncoder {
            return Vec::new();
        }
        self.refused
            .get_or_init(|| {
                ensure_com();
                activate_codec(self.kind, self.codec, self.index, &self.name)
                    .map(|transform| super::mf_encoder_sync::probe_refused(&transform))
                    .unwrap_or_default()
            })
            .clone()
    }

    fn offers(&self) -> Vec<CodecOffer> {
        let hardware = self.kind == Kind::HardwareEncoder;
        if self.codec != CodecKind::H264 {
            return if MF_VIDEO_ENCODERS_CONNECTED {
                vec![CodecOffer::video(self.codec, false, hardware)]
            } else {
                Vec::new()
            };
        }
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
    /// Plan 4.10a for hybrid laptops (merge W1): an admitted capture runs on
    /// one adapter; a hardware MFT of another adapter gives way while that
    /// adapter has an MFT of the codec (the texture path stays on one GPU),
    /// with the reason; `None` opens.
    fn other_adapter(&self, admitted: bool) -> Option<String> {
        if !admitted || self.kind != Kind::HardwareEncoder {
            return None;
        }
        let capture = super::mf_encoder::d3d_input::capture_adapter()?;
        let entries = mf_enum::found().encoders(self.codec);
        prefer_capture_adapter(self.adapter, capture, entries).then(|| {
            format!(
                "{}: the capture's adapter has its own encoder of {}",
                self.name,
                self.codec.key()
            )
        })
    }

    fn create(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: Option<CaptureGrant>,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        if choice.codec != self.codec
            || (self.codec != CodecKind::H264 && !MF_VIDEO_ENCODERS_CONNECTED)
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("{} does not encode {}", self.name, choice.codec.key()),
            ));
        }
        let profile = match (self.codec, choice.profile) {
            (CodecKind::H264, Some(profile)) => profile,
            (CodecKind::H264, None) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "H.264 needs a profile",
                ))
            }
            // HEVC and AV1: 4:2:0 Main only (`profile` High would be 4:4:4,
            // which these MFTs do not offer); `codec` names the stream.
            (_, None) => H264Profile::High,
            (_, Some(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("{} encodes 4:2:0 only", self.name),
                ))
            }
        };
        if let Some(reason) = self.other_adapter(grant.is_some()) {
            return Err(io::Error::new(io::ErrorKind::Unsupported, reason));
        }
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
        let transform = activate_codec(self.kind, self.codec, self.index, &self.name)?;
        let setup = EncoderSetup {
            name: self.name.clone(),
            profile,
            size,
            fps: fps.max(1),
            codec: self.codec,
            adapter: self.adapter,
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
    codec: CodecKind,
    index: usize,
    name: String,
    caps: OnceLock<Vec<CodecOffer>>,
}

impl DecoderFactory for MfDecoderFactory {
    fn name(&self) -> String {
        self.name.clone()
    }

    /// The decoder opens on D3D11VA where this host's GPU decodes the codec
    /// (BK5w) and falls back to software itself, so it lists both stages:
    /// the viewer's choice „Dekodierer“ (BK6, 4.10) keeps one of them.
    fn offers(&self) -> Vec<CodecOffer> {
        self.caps
            .get_or_init(|| {
                let hardware = super::mf_decoder::hardware_offered(self.codec);
                let mut offers = Vec::new();
                for stage in [true, false].into_iter().filter(|on| hardware || !on) {
                    if self.codec == CodecKind::H264 {
                        for profile in [H264Profile::High, H264Profile::ConstrainedBaseline] {
                            offers.push(CodecOffer::h264(profile, stage).with_max(FrameSize::MAX));
                        }
                    } else {
                        offers.push(
                            CodecOffer::video(self.codec, false, stage).with_max(FrameSize::MAX),
                        );
                    }
                }
                if self.codec == CodecKind::H264 {
                    offers.extend(confirmed_offers(
                        Kind::Decoder,
                        self.index,
                        &self.name,
                        hardware,
                    ));
                }
                offers
            })
            .clone()
    }

    fn open(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        // The viewer's choice „Dekodierer“ (BK6, 4.10).
        let preference = crate::screen::core::negotiate::decoder_preference();
        Ok(Box::new(MfDecoder::open(
            self.index, &self.name, header, preference,
        )?))
    }

    /// B-36: HEVC and AV1 name their references explicitly, H.264 not
    /// (`mf_decoder::keeps_references`).
    fn keeps_references(&self, codec: CodecKind) -> bool {
        codec == self.codec && super::mf_decoder::keeps_references(codec)
    }
}
