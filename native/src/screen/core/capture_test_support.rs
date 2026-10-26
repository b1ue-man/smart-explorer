//! Source fixtures for the single remote task suite; never used by production.
use std::sync::Arc;
use super::capture_admission::{CaptureGeneration, CaptureGrant, CaptureOwner,
    CapturePlan, SessionAdmission, SourceKey};
use super::capture_image::{CaptureImageBuilder, CapturedImage};
use super::capture_plan::CapturePhases;
use super::frame_budget::{FrameContract, FrameLimits, MAX_WORKING_BYTES};
use super::frame_memory::{CapacityCeiling, MemoryFacts, MemoryPool};
use super::host_codec_adapters::{HostCodecPlan, HostContract, HostInputPixels, HostOutputPlan};
use super::pixel_convert::{BgraImage, FrameSize};

pub(crate) fn session(contract: Option<FrameContract>) -> Arc<SessionAdmission> {
    let facts = Some(MemoryFacts {available_physical: 1 << 40,
        available_commit: 1 << 40, available_address_space: 1 << 40});
    let pool = Arc::new(MemoryPool::default());
    let admission = pool.admit_session(facts, CapacityCeiling::desktop(facts),
        contract.map_or(MAX_WORKING_BYTES, |contract| contract.limits.working_bytes),
        "capture-fixture").unwrap();
    admission.bind_contract(contract).unwrap();
    admission
}

pub(crate) fn grant(size: FrameSize, contract: Option<FrameContract>) -> CaptureGrant {
    let admission = session(contract);
    let key = SourceKey::new(admission.session(), "pixels").unwrap();
    let mut plan = CapturePlan::new(key, CaptureGeneration::new(1));
    plan.bind_contract(contract).unwrap();
    plan.add_bgra(CaptureOwner::SourcePixels, size, 2, CapturePhases::ALL).unwrap();
    let limits = contract.map_or(FrameLimits::LEGACY, |contract| contract.limits);
    let output = HostOutputPlan::new(contract.map_or(HostContract::Legacy,
        HostContract::Versioned), limits.fit(limits.max_size), HostCodecPlan::Tiles).unwrap();
    output.append_to(&mut plan, HostInputPixels::SeparateImage).unwrap();
    admission.authority().approve(plan).unwrap()
}

pub(crate) fn image(source: &BgraImage, grant: &CaptureGrant) -> CapturedImage {
    let mut image = CaptureImageBuilder::new(source.size(), FrameLimits::RESOURCE,
        grant, CaptureOwner::SourcePixels).unwrap();
    image.copy_from(&source.view()).unwrap();
    image.freeze()
}
