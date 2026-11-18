//! The route decisions of the encoder's texture input (moved out of
//! `mf_d3d_input.rs` unchanged by the fix of 2026-11-18, which keeps that
//! file under 500 lines): why a hardware encoder takes no textures of the
//! capture (`SystemReason`, `encoder_route`, decision B-20) and the switch
//! of the GPU path (`GpuSwitch`).
//!
//! `GpuSwitch` (fix of 2026-11-18, host log of a Windows VM with an NVIDIA
//! GPU): a GPU failure switched the GPU path off for the whole host process,
//! and an encoder that met the turnover of a capture path (Desktop
//! Duplication and Graphics Capture each have their own device and grant)
//! switched it off for a failure that belonged to the capture before. The
//! switch now holds for the capture device the failure belongs to: a
//! failure of an earlier device leaves the current one on, and a new capture
//! device (another capture path, a rebuilt duplication) tries the GPU path
//! again. Every change goes to the host log with its reason.

/// Why a hardware encoder takes no textures of the capture's device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::screen::os) enum SystemReason {
    Disabled,
    /// No capture device seen yet in this process.
    NoCapture,
    NoVideo,
    /// The MFT does not report `MF_SA_D3D11_AWARE`.
    NotD3D11Aware,
    /// MFT and capture on different adapters (hybrid laptops).
    OtherAdapter,
    ManagerRefused,
    /// BK12: the capture's device is not the vendor encoder's GPU.
    OtherVendor,
}

/// What the encoder side knows when a hardware MFT opens.
#[derive(Clone, Copy, Debug)]
pub(in crate::screen::os) struct EncoderFacts {
    pub disabled: bool,
    pub capture: bool,
    pub video: bool,
    pub aware: bool,
    pub capture_adapter: Option<u64>,
    pub encoder_adapter: Option<u64>,
}

/// Decision B-20 at open: textures only into an MFT that is D3D11-aware and
/// on the capture's adapter (an MFT without the LUID attribute is tried; its
/// refusal of the manager keeps system memory).
pub(in crate::screen::os) fn encoder_route(facts: EncoderFacts) -> Result<(), SystemReason> {
    let adapters = (facts.capture_adapter, facts.encoder_adapter);
    let other_adapter = matches!(adapters, (Some(capture), Some(encoder)) if capture != encoder);
    [
        (facts.disabled, SystemReason::Disabled),
        (!facts.capture, SystemReason::NoCapture),
        (!facts.video, SystemReason::NoVideo),
        (!facts.aware, SystemReason::NotD3D11Aware),
        (other_adapter, SystemReason::OtherAdapter),
    ]
    .into_iter()
    .find_map(|(refused, reason)| refused.then_some(reason))
    .map_or(Ok(()), Err)
}

/// What a GPU failure did to the switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::screen::os) enum Switched {
    /// The GPU path is off for the current capture device from now on.
    Off,
    /// It was off for the current capture device already.
    AlreadyOff,
    /// The failure belongs to an earlier capture device: the current one
    /// keeps the GPU path.
    Earlier,
}

/// The GPU path of this process: off for one capture device (identity:
/// its `ID3D11Device` pointer; `None` before any capture) with the reason.
#[derive(Debug, Default, PartialEq, Eq)]
pub(in crate::screen::os) struct GpuSwitch {
    off: Option<(Option<usize>, String)>,
}

impl GpuSwitch {
    pub(in crate::screen::os) const fn new() -> Self {
        Self { off: None }
    }

    /// Whether the GPU path is off (for the current capture device: a new
    /// device ends the switch in `capture_on`).
    pub(in crate::screen::os) fn is_off(&self) -> bool {
        self.off.is_some()
    }

    /// A GPU failure of the device `failed` (the armed encoder's device,
    /// else the capture's) while the capture reads on `current`.
    pub(in crate::screen::os) fn fail(
        &mut self,
        failed: Option<usize>,
        current: Option<usize>,
        reason: &str,
    ) -> Switched {
        if failed != current {
            return Switched::Earlier;
        }
        if self.off.is_some() {
            return Switched::AlreadyOff;
        }
        self.off = Some((current, reason.to_string()));
        Switched::Off
    }

    /// The capture reads on `device` now: a switch that was off for another
    /// device ends; its reason is returned for the host log.
    pub(in crate::screen::os) fn capture_on(&mut self, device: usize) -> Option<String> {
        match &self.off {
            Some((raw, _)) if *raw != Some(device) => self.off.take().map(|(_, reason)| reason),
            _ => None,
        }
    }
}
