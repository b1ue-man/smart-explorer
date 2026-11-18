//! Rank and fallback of the viewer's H.264 decoder stages (plan
//! `bildkette.md` BK5w, B-37 part 1, 4.10 „Dekodierer“): hardware (D3D11VA
//! through the Media Foundation decoder with a DXGI device manager), then
//! the same Media Foundation decoder in software, then OpenH264. The stage
//! is chosen when a decoder opens, with the profile and size of the stream;
//! a hardware decoder that fails while it runs (device removed, a format
//! change the hardware does not take, an output that cannot be read) is
//! remembered here, so the decoder the viewer opens for the requested key
//! frame starts at the next stage and the session goes on.
//!
//! This file holds only the decisions; it imports nothing of Windows, so
//! every rule is testable without a GPU or Media Foundation. OpenH264 is a
//! separate factory of the codec register (`codec_inventory` puts it after
//! Media Foundation); `ranking` names it so the whole order is in one place,
//! and `mf_stages` leaves it out for the Media Foundation decoder.
use std::sync::{Mutex, PoisonError};

use crate::screen::core::decoder_health::{for_session, session_epoch};
use crate::screen::core::negotiate::CodecKind;

/// The user's choice „Dekodierer: Automatisch / Hardware / Software“
/// (4.10), set by the viewer (BK6, `negotiate::set_decoder_preference`):
/// `Automatic` is hardware, then software, each on failure; `Hardware`
/// only hardware (no hardware means the Media Foundation decoder refuses);
/// `Software` the Media Foundation decoder without a device, then OpenH264.
pub use crate::screen::core::negotiate::DecoderPreference;

/// One stage of the order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeStage {
    /// Media Foundation with a D3D11 device manager (DXVA, D3D11VA).
    Hardware,
    /// Media Foundation on the CPU.
    SoftwareMft,
    /// OpenH264 (only after the user's consent).
    OpenH264,
}

impl DecodeStage {
    /// Key for logs, measurements and JSON.
    pub fn key(self) -> &'static str {
        match self {
            Self::Hardware => "hardware",
            Self::SoftwareMft => "software_mft",
            Self::OpenH264 => "openh264",
        }
    }

    /// Text of the statistics („Dekodierer: Hardware“).
    pub fn label(self) -> &'static str {
        match self {
            Self::Hardware => "Hardware (D3D11VA)",
            Self::SoftwareMft => "Software (Media Foundation)",
            Self::OpenH264 => "Software (OpenH264)",
        }
    }

    pub fn hardware(self) -> bool {
        self == Self::Hardware
    }
}

/// The stages of `preference` in their order.
pub fn ranking(preference: DecoderPreference) -> &'static [DecodeStage] {
    match preference {
        DecoderPreference::Automatic => &[
            DecodeStage::Hardware,
            DecodeStage::SoftwareMft,
            DecodeStage::OpenH264,
        ],
        DecoderPreference::Hardware => &[DecodeStage::Hardware],
        DecoderPreference::Software => &[DecodeStage::SoftwareMft, DecodeStage::OpenH264],
    }
}

/// Why the hardware stage was given up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwareFailure {
    /// The device was removed, reset or hung (`DXGI_ERROR_DEVICE_*`).
    DeviceLost,
    /// The stream changed to a format the hardware does not take.
    FormatUnsupported,
    /// The decoder's texture could not be read back.
    OutputUnusable,
    /// The hardware refused to open for one stream size.
    OpenRefused,
}

impl HardwareFailure {
    pub fn key(self) -> &'static str {
        match self {
            Self::DeviceLost => "device_lost",
            Self::FormatUnsupported => "format_unsupported",
            Self::OutputUnusable => "output_unusable",
            Self::OpenRefused => "open_refused",
        }
    }
}

/// `DXGI_ERROR_DEVICE_REMOVED` (checked against the `windows` crate in
/// `mf_decoder_d3d.rs`).
pub const DXGI_DEVICE_REMOVED: i32 = 0x887A_0005_u32 as i32;
/// `DXGI_ERROR_DEVICE_HUNG`.
pub const DXGI_DEVICE_HUNG: i32 = 0x887A_0006_u32 as i32;
/// `DXGI_ERROR_DEVICE_RESET`.
pub const DXGI_DEVICE_RESET: i32 = 0x887A_0007_u32 as i32;
/// `DXGI_ERROR_DRIVER_INTERNAL_ERROR`.
pub const DXGI_DRIVER_INTERNAL_ERROR: i32 = 0x887A_0020_u32 as i32;
/// `MF_E_UNSUPPORTED_D3D_TYPE`.
pub const MF_UNSUPPORTED_D3D_TYPE: i32 = 0xC00D_6D76_u32 as i32;
/// `MF_E_UNSUPPORTED_MEDIATYPE_AT_D3D_FEATURE_LEVEL`.
pub const MF_UNSUPPORTED_AT_FEATURE_LEVEL: i32 = 0xC00D_3709_u32 as i32;

/// The hardware failure an error of a running hardware decoder means;
/// `None` for an error that is not the hardware's (a broken bitstream: the
/// key frame request alone repairs it). `device_lost` is the answer of
/// `GetDeviceRemovedReason` after the error; `noted` a failure the decoder
/// recognised itself (stream change, readback).
pub fn classify(
    code: Option<i32>,
    device_lost: bool,
    noted: Option<HardwareFailure>,
) -> Option<HardwareFailure> {
    if device_lost {
        return Some(HardwareFailure::DeviceLost);
    }
    match code {
        Some(
            DXGI_DEVICE_REMOVED | DXGI_DEVICE_HUNG | DXGI_DEVICE_RESET | DXGI_DRIVER_INTERNAL_ERROR,
        ) => Some(HardwareFailure::DeviceLost),
        Some(MF_UNSUPPORTED_D3D_TYPE | MF_UNSUPPORTED_AT_FEATURE_LEVEL) => {
            Some(HardwareFailure::FormatUnsupported)
        }
        _ => noted,
    }
}

/// Stream sizes whose hardware open failed, before all hardware is given up.
pub const MAX_REFUSED_SIZES: usize = 8;

/// What this process learned about the hardware stage. A failure in
/// operation ends the hardware stage until the next viewer session opens
/// (fix of 2026-11-18; before: the rest of the process); a refusal at open
/// ends it for that stream size.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StageMemory {
    demoted: Option<HardwareFailure>,
    refused: Vec<(u32, u32)>,
}

impl StageMemory {
    pub const fn new() -> Self {
        Self {
            demoted: None,
            refused: Vec::new(),
        }
    }

    /// Whether hardware may be tried for a stream of `size`.
    pub fn allows_hardware(&self, size: (u32, u32)) -> bool {
        self.demoted.is_none() && !self.refused.contains(&size)
    }

    /// Why the hardware stage ended, when it did.
    #[cfg_attr(not(test), allow(dead_code))] // for the statistics of BK6
    pub fn demoted(&self) -> Option<HardwareFailure> {
        self.demoted
    }

    /// Notes `failure` of a hardware decoder of `size`.
    pub fn record(&mut self, failure: HardwareFailure, size: (u32, u32)) {
        if failure != HardwareFailure::OpenRefused {
            self.demoted = Some(failure);
            return;
        }
        if !self.refused.contains(&size) {
            self.refused.push(size);
        }
        if self.refused.len() >= MAX_REFUSED_SIZES {
            self.demoted = Some(HardwareFailure::OpenRefused);
        }
    }
}

/// What this process learned about the hardware stage, per codec (plan
/// 2026-11-17 P9: a failure of one codec's hardware decoder ends the
/// hardware stage of that codec only, never hardware decoding in general).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodecMemory {
    h264: StageMemory,
    hevc: StageMemory,
    av1: StageMemory,
}

impl CodecMemory {
    pub const fn new() -> Self {
        Self {
            h264: StageMemory::new(),
            hevc: StageMemory::new(),
            av1: StageMemory::new(),
        }
    }

    fn slot(&mut self, codec: CodecKind) -> Option<&mut StageMemory> {
        match codec {
            CodecKind::H264 => Some(&mut self.h264),
            CodecKind::Hevc => Some(&mut self.hevc),
            CodecKind::Av1 => Some(&mut self.av1),
            CodecKind::Tiles => None,
        }
    }

    /// What is known of `codec`'s hardware stage (nothing for the tiles).
    pub fn of(&self, codec: CodecKind) -> StageMemory {
        match codec {
            CodecKind::H264 => self.h264.clone(),
            CodecKind::Hevc => self.hevc.clone(),
            CodecKind::Av1 => self.av1.clone(),
            CodecKind::Tiles => StageMemory::new(),
        }
    }

    /// Notes `failure` of `codec`'s hardware decoder of `size`.
    pub fn record(&mut self, codec: CodecKind, failure: HardwareFailure, size: (u32, u32)) {
        if let Some(memory) = self.slot(codec) {
            memory.record(failure, size);
        }
    }
}

/// What this process learned about the hardware stage, with the viewer
/// session it learned it in (fix of 2026-11-18: a newer viewer session
/// starts without it, `decoder_health::for_session`).
static MEMORY: Mutex<(u64, CodecMemory)> = Mutex::new((0, CodecMemory::new()));

/// A copy of the memory of `codec` for the newest viewer session.
pub fn known(codec: CodecKind) -> StageMemory {
    let mut memory = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
    for_session(&mut *memory, session_epoch()).of(codec)
}

/// Notes `failure` of `codec`'s hardware decoder of `size` until the next
/// viewer session opens.
pub fn remember(codec: CodecKind, failure: HardwareFailure, size: (u32, u32)) {
    let mut memory = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
    for_session(&mut *memory, session_epoch()).record(codec, failure, size);
}

/// The Media Foundation stages a decoder of `size` tries in order:
/// `hardware_fits` is the device's answer for the stream's profile and size
/// (`D3D11_DECODER_PROFILE_H264_VLD_NOFGT`, NV12, a decoder configuration
/// for the size).
pub fn mf_stages(
    preference: DecoderPreference,
    hardware_fits: bool,
    memory: &StageMemory,
    size: (u32, u32),
) -> Vec<DecodeStage> {
    ranking(preference)
        .iter()
        .copied()
        .filter(|stage| match stage {
            DecodeStage::Hardware => hardware_fits && memory.allows_hardware(size),
            DecodeStage::SoftwareMft => true,
            DecodeStage::OpenH264 => false,
        })
        .collect()
}

/// The first stage of `preference` that this process can open for `size`
/// (the whole order including OpenH264, as the codec register applies it).
#[cfg_attr(not(test), allow(dead_code))] // the register applies it (BK6 names it)
pub fn first_stage(
    preference: DecoderPreference,
    hardware_fits: bool,
    media_foundation: bool,
    openh264: bool,
    memory: &StageMemory,
    size: (u32, u32),
) -> Option<DecodeStage> {
    ranking(preference)
        .iter()
        .copied()
        .find(|stage| match stage {
            DecodeStage::Hardware => {
                media_foundation && hardware_fits && memory.allows_hardware(size)
            }
            DecodeStage::SoftwareMft => media_foundation,
            DecodeStage::OpenH264 => openh264,
        })
}

/// The decoder's name for statistics and measurements: the transform and
/// the stage it runs on.
pub fn stage_name(transform: &str, stage: DecodeStage) -> String {
    format!("{transform} [{}]", stage.label())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HD: (u32, u32) = (1920, 1080);
    const QHD: (u32, u32) = (2560, 1440);

    /// 4.10a: hardware, then the software MFT, then OpenH264; „Hardware“
    /// and „Software“ narrow the order, the default is automatic.
    #[test]
    fn bildkette_task_win_decode_ranking() {
        use DecodeStage::*;
        assert_eq!(DecoderPreference::default(), DecoderPreference::Automatic);
        assert_eq!(
            ranking(DecoderPreference::Automatic),
            &[Hardware, SoftwareMft, OpenH264]
        );
        assert_eq!(ranking(DecoderPreference::Hardware), &[Hardware]);
        assert_eq!(
            ranking(DecoderPreference::Software),
            &[SoftwareMft, OpenH264]
        );
    }

    /// The stage is chosen at open from the device's answer for the
    /// stream's profile and size; the Media Foundation decoder never names
    /// OpenH264 (that is the next factory of the register).
    #[test]
    fn bildkette_task_win_decode_open_stages() {
        use DecodeStage::*;
        let fresh = StageMemory::new();
        let auto = DecoderPreference::Automatic;
        assert_eq!(
            mf_stages(auto, true, &fresh, HD),
            vec![Hardware, SoftwareMft]
        );
        assert_eq!(mf_stages(auto, false, &fresh, HD), vec![SoftwareMft]);
        assert_eq!(
            mf_stages(DecoderPreference::Hardware, true, &fresh, HD),
            vec![Hardware]
        );
        assert!(mf_stages(DecoderPreference::Hardware, false, &fresh, HD).is_empty());
        assert_eq!(
            mf_stages(DecoderPreference::Software, true, &fresh, HD),
            vec![SoftwareMft]
        );
        // The whole order as the register applies it.
        assert_eq!(
            first_stage(auto, true, true, true, &fresh, HD),
            Some(Hardware)
        );
        assert_eq!(
            first_stage(auto, false, true, true, &fresh, HD),
            Some(SoftwareMft)
        );
        assert_eq!(
            first_stage(auto, true, false, true, &fresh, HD),
            Some(OpenH264)
        );
        assert_eq!(first_stage(auto, true, false, false, &fresh, HD), None);
        assert_eq!(
            first_stage(DecoderPreference::Software, true, true, true, &fresh, HD),
            Some(SoftwareMft)
        );
    }

    /// A removed device while decoding: the next decoder (opened for the
    /// requested key frame) starts at the software MFT, for every size.
    #[test]
    fn bildkette_task_win_decode_device_removed_falls_back() {
        use DecodeStage::*;
        let mut memory = StageMemory::new();
        let failure = classify(Some(DXGI_DEVICE_REMOVED), false, None);
        assert_eq!(failure, Some(HardwareFailure::DeviceLost));
        memory.record(HardwareFailure::DeviceLost, HD);
        assert_eq!(memory.demoted(), Some(HardwareFailure::DeviceLost));
        assert!(!memory.allows_hardware(HD));
        assert!(!memory.allows_hardware(QHD));
        let auto = DecoderPreference::Automatic;
        assert_eq!(mf_stages(auto, true, &memory, HD), vec![SoftwareMft]);
        assert_eq!(
            first_stage(auto, true, true, true, &memory, QHD),
            Some(SoftwareMft)
        );
        // Forced hardware has nothing left; the viewer keeps its session and
        // reports the refusal of the next open.
        assert!(mf_stages(DecoderPreference::Hardware, true, &memory, HD).is_empty());
    }

    /// Which errors end the hardware stage and which do not.
    #[test]
    fn bildkette_task_win_decode_classify() {
        let lost = Some(HardwareFailure::DeviceLost);
        for code in [
            DXGI_DEVICE_REMOVED,
            DXGI_DEVICE_HUNG,
            DXGI_DEVICE_RESET,
            DXGI_DRIVER_INTERNAL_ERROR,
        ] {
            assert_eq!(classify(Some(code), false, None), lost);
        }
        let format = Some(HardwareFailure::FormatUnsupported);
        assert_eq!(classify(Some(MF_UNSUPPORTED_D3D_TYPE), false, None), format);
        assert_eq!(
            classify(Some(MF_UNSUPPORTED_AT_FEATURE_LEVEL), false, None),
            format
        );
        // A stream change without NV12 and a failed readback, as the
        // decoder notes them itself.
        assert_eq!(classify(None, false, format), format);
        assert_eq!(
            classify(
                Some(0x8000_4005_u32 as i32),
                false,
                Some(HardwareFailure::OutputUnusable)
            ),
            Some(HardwareFailure::OutputUnusable)
        );
        // `GetDeviceRemovedReason` after any error wins.
        assert_eq!(classify(Some(0x8000_4005_u32 as i32), true, None), lost);
        // A broken bitstream (E_FAIL, MF_E_INVALIDSTREAMNUMBER) keeps hardware.
        assert_eq!(classify(Some(0x8000_4005_u32 as i32), false, None), None);
        assert_eq!(classify(Some(0xC00D_36B3_u32 as i32), false, None), None);
        assert_eq!(classify(None, false, None), None);
    }

    /// A refusal at open ends hardware for that size only, until
    /// `MAX_REFUSED_SIZES` sizes failed.
    #[test]
    fn bildkette_task_win_decode_open_refusal_per_size() {
        let mut memory = StageMemory::new();
        memory.record(HardwareFailure::OpenRefused, HD);
        memory.record(HardwareFailure::OpenRefused, HD);
        assert!(!memory.allows_hardware(HD));
        assert!(memory.allows_hardware(QHD));
        assert_eq!(memory.demoted(), None);
        for width in 0..MAX_REFUSED_SIZES as u32 {
            memory.record(HardwareFailure::OpenRefused, (640 + 16 * width, 360));
        }
        assert_eq!(memory.demoted(), Some(HardwareFailure::OpenRefused));
        assert!(!memory.allows_hardware(QHD));
    }

    /// P9: a failure of one codec's hardware decoder ends the hardware
    /// stage of that codec only.
    #[test]
    fn p9_task_win_decode_hardware_memory_per_codec() {
        let mut memory = CodecMemory::new();
        memory.record(CodecKind::Av1, HardwareFailure::DeviceLost, HD);
        memory.record(CodecKind::Tiles, HardwareFailure::DeviceLost, HD);
        assert!(!memory.of(CodecKind::Av1).allows_hardware(QHD));
        assert!(memory.of(CodecKind::Hevc).allows_hardware(HD));
        assert!(memory.of(CodecKind::H264).allows_hardware(HD));
        memory.record(CodecKind::H264, HardwareFailure::OpenRefused, HD);
        assert!(!memory.of(CodecKind::H264).allows_hardware(HD));
        assert!(memory.of(CodecKind::H264).allows_hardware(QHD));
        assert!(memory.of(CodecKind::Hevc).allows_hardware(HD));
    }

    /// The statistics and measurements name the stage.
    #[test]
    fn bildkette_task_win_decode_stage_name() {
        let name = stage_name("Microsoft H264 Video Decoder MFT", DecodeStage::Hardware);
        assert_eq!(
            name,
            "Microsoft H264 Video Decoder MFT [Hardware (D3D11VA)]"
        );
        assert!(
            stage_name("x", DecodeStage::SoftwareMft).ends_with("[Software (Media Foundation)]")
        );
        assert!(DecodeStage::Hardware.hardware());
        assert!(!DecodeStage::SoftwareMft.hardware());
        assert!(!DecodeStage::OpenH264.hardware());
        let keys = [
            DecodeStage::Hardware.key(),
            DecodeStage::SoftwareMft.key(),
            DecodeStage::OpenH264.key(),
        ];
        assert_eq!(keys, ["hardware", "software_mft", "openh264"]);
        assert_eq!(DXGI_DEVICE_REMOVED as u32, 0x887A_0005);
        assert_eq!(MF_UNSUPPORTED_D3D_TYPE as u32, 0xC00D_6D76);
    }
}
