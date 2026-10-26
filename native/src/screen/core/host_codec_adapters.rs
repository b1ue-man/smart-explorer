//! Portable product-buffer bounds for the selected host codec and snapshots.
use super::capture_plan::{bgra_bytes, CaptureOwner, CapturePhases, CapturePlan, HostPhase};
use super::frame_budget::{FrameContract, FrameLimits};
use super::frame_size_caps::{PacketBudget, RECORD_BUFFER_OVERHEAD};
use super::pixel_convert::FrameSize;
use super::wire::invalid;
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostContract {
    Legacy,
    Versioned(FrameContract),
}
impl HostContract {
    pub(crate) fn frame_contract(self) -> Option<FrameContract> {
        match self {
            Self::Legacy => None,
            Self::Versioned(contract) => Some(contract),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostCodecPlan {
    Tiles,
    OpenH264,
    /// The platform reports the actual cbSize/fallback before allocation.
    MediaFoundation {
        output_buffer_bytes: u64,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostInputPixels {
    BorrowedSource,
    SeparateImage,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostOutputSide {
    InProcess,
    Worker,
    Parent,
}
#[derive(Clone, Debug)]
pub(crate) struct HostOutputPlan {
    contract: HostContract,
    size: FrameSize,
    codec: HostCodecPlan,
    include_output: bool,
    side: HostOutputSide,
    raw_bound: Option<u64>,
    legacy_limits: Option<FrameLimits>,
}
impl HostOutputPlan {
    /// Native metadata preparation does not validate against a legacy encoder
    /// side limit. The source planner still computes every native/band owner;
    /// this value authorizes no output allocation or codec opening.
    pub(crate) fn for_source(
        contract: HostContract,
        native_size: FrameSize,
        capture_limits: FrameLimits,
    ) -> io::Result<Self> {
        if let HostContract::Versioned(contract) = contract {
            contract.validate()?;
        }
        capture_limits.validate_capture()?;
        bgra_bytes(native_size)?;
        capture_limits.check(capture_limits.fit(native_size))?;
        Ok(Self {
            contract,
            size: native_size,
            codec: HostCodecPlan::Tiles,
            include_output: false,
            side: HostOutputSide::InProcess,
            raw_bound: None,
            legacy_limits: None,
        })
    }
    pub(crate) fn new(
        contract: HostContract,
        size: FrameSize,
        codec: HostCodecPlan,
    ) -> io::Result<Self> {
        if let HostContract::Versioned(contract) = contract {
            contract.validate()?;
        }
        contract
            .frame_contract()
            .map_or(FrameLimits::LEGACY, |c| c.limits)
            .check(size)?;
        if let HostCodecPlan::MediaFoundation {
            output_buffer_bytes,
        } = codec
        {
            if output_buffer_bytes == 0 || output_buffer_bytes > 64 * 1024 * 1024 {
                return Err(invalid(
                    "MF output allocation exceeds existing 64 MiB bound",
                ));
            }
        }
        Ok(Self {
            contract,
            size,
            codec,
            include_output: true,
            side: HostOutputSide::InProcess,
            raw_bound: None,
            legacy_limits: None,
        })
    }
    /// Private host output geometry, independent of v1's unchanged wire format.
    pub(crate) fn with_legacy_limits(mut self, limits: FrameLimits) -> io::Result<Self> {
        if self.contract != HostContract::Legacy || limits.intersect(FrameLimits::LEGACY) != limits
        {
            return Err(invalid("local legacy output exceeds its protocol class"));
        }
        limits.check(self.size)?;
        PacketBudget::for_receive(None, Some(limits))?;
        self.legacy_limits = Some(limits);
        Ok(self)
    }
    pub(crate) fn legacy_limits(&self) -> Option<FrameLimits> {
        self.legacy_limits
    }
    pub(crate) fn limits(&self) -> FrameLimits {
        self.contract
            .frame_contract()
            .map(|contract| contract.limits)
            .or(self.legacy_limits)
            .unwrap_or(FrameLimits::LEGACY)
    }
    pub(crate) fn contract(&self) -> HostContract {
        self.contract
    }
    pub(crate) fn size(&self) -> FrameSize {
        self.size
    }
    pub(crate) fn codec(&self) -> HostCodecPlan {
        self.codec
    }
    pub(crate) fn source_only(&self) -> Self {
        Self {
            include_output: false,
            ..self.clone()
        }
    }
    pub(crate) fn for_worker(&self) -> Self {
        Self {
            side: HostOutputSide::Worker,
            ..self.clone()
        }
    }
    pub(crate) fn for_parent(&self) -> Self {
        Self {
            side: HostOutputSide::Parent,
            ..self.clone()
        }
    }
    pub(crate) fn for_parent_of(&self, source: &CapturePlan) -> Self {
        Self {
            side: HostOutputSide::Parent,
            raw_bound: source.source_image_bytes(),
            ..self.clone()
        }
    }
    pub(crate) fn reconfigure(&self, size: FrameSize, codec: HostCodecPlan) -> io::Result<Self> {
        let output = Self::new(self.contract, size, codec)?;
        match self.legacy_limits {
            Some(limits) => output.with_legacy_limits(limits),
            None => Ok(output),
        }
    }

    /// Adds local host owners only. A local viewer keeps its independent W.
    /// Native metadata, pending old samples, and resize generations are added
    /// by the prepared source as actual separate owners, before approval.
    pub(crate) fn append_to(
        &self,
        source: &mut CapturePlan,
        input: HostInputPixels,
    ) -> io::Result<()> {
        if !self.include_output {
            return Ok(());
        }
        let raw_bound = self.raw_bound.or_else(|| source.source_image_bytes());
        let configured = Self {
            raw_bound,
            ..self.clone()
        };
        let mut selected = CapturePlan::new(source.source().clone(), source.generation());
        configured.append_selected(&mut selected, input)?;
        if self.codec == HostCodecPlan::Tiles {
            return source.add_member(selected);
        }
        let mut alternatives = CapturePlan::new(source.source().clone(), source.generation());
        alternatives.add_serial_member(
            CapturePlan::new(source.source().clone(), source.generation()),
            selected,
        )?;
        let tiles = Self {
            codec: HostCodecPlan::Tiles,
            ..configured
        };
        let mut fallback = CapturePlan::new(source.source().clone(), source.generation());
        tiles.append_selected(&mut fallback, input)?;
        alternatives.add_serial_member(
            CapturePlan::new(source.source().clone(), source.generation()),
            fallback,
        )?;
        source.add_member(alternatives)
    }
}

#[path = "host_codec_phase_plan.rs"]
mod phases;
#[path = "admitted_tiles_encoder.rs"]
mod tiles;
pub(crate) use tiles::AdmittedTilesEncoder;
