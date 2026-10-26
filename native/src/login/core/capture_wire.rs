//! Private, inherited helper-channel capture control. No network authority.
use std::io;
use serde::{Deserialize, Serialize};
use crate::screen::core::{
    capture_admission::{CaptureGeneration, SourceKey},
    capture_plan::{CaptureOwner, CapturePhases, CapturePlan, HostPhase, OWNER_COUNT, PHASE_COUNT},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Admission {pub session: String, pub working: u64}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    pub source: String,
    pub generation: u64,
    pub phases: [[u64; OWNER_COUNT]; PHASE_COUNT],
    pub raw_bytes: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(crate) enum CaptureCall {
    Prepare {cookie: String, id: u64, plan: Plan},
    Retire {cookie: String, id: u64},
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GrantReply {pub id: u64, pub approved: bool, pub plan: Plan}

const OWNERS: [CaptureOwner; OWNER_COUNT] = [
    CaptureOwner::NativeSurface, CaptureOwner::Readback, CaptureOwner::SourcePixels,
    CaptureOwner::CompositionPixels, CaptureOwner::ScaledPixels, CaptureOwner::FilterScratch,
    CaptureOwner::Metadata, CaptureOwner::Transport, CaptureOwner::CodecInput, CaptureOwner::CodecOutput,
];
const PHASES: [HostPhase; PHASE_COUNT] = [HostPhase::Capture, HostPhase::LiveEncode,
    HostPhase::EncodedTransfer, HostPhase::RawSnapshot, HostPhase::SnapshotEncode,
    HostPhase::Send, HostPhase::Convert, HostPhase::CodecOutput, HostPhase::Transition];

impl Plan {
    pub(crate) fn from_plan(plan: &CapturePlan) -> io::Result<Self> {
        // This channel carries one native login source, never family plans or
        // host codecs. Aggregation therefore preserves the exact phase table.
        if plan.concurrency_cases().len() != 1
            || plan.requirements().iter().any(|r| r.source() != plan.source() || r.generation() != plan.generation())
            || plan.contract_binding().is_some_and(|binding| binding.is_some()) {
            return Err(invalid("login source plan is not a native single-source plan"));
        }
        let raw_bytes = plan.requirements().iter().filter(|r| r.owner() == CaptureOwner::SourcePixels)
            .map(|r| r.bytes_each()).max().unwrap_or(0);
        let descriptor = Self {source: plan.source().source().into(), generation: plan.generation().value(),
            phases: *plan.phase_table(), raw_bytes};
        descriptor.check()?;
        Ok(descriptor)
    }
    fn check(&self) -> io::Result<()> {
        if !self.source.starts_with("login:") || self.generation == 0
            || self.raw_bytes > super::model::MAX_PIXELS as u64
            || self.phases.iter().any(|row| row[CaptureOwner::Transport as usize] != 0) {
            return Err(invalid("login capture source or disjoint raw credit is invalid"));
        }
        // X11 cursor header/reply claims are supplemental to the active
        // source grant. They never mint a BGRA source or parent pixel slot.
        if self.raw_bytes == 0 && self.phases.iter().any(|row| row.iter().enumerate().any(|(owner, bytes)|
            *bytes != 0 && owner != CaptureOwner::Readback as usize && owner != CaptureOwner::Metadata as usize)) {
            return Err(invalid("login supplemental grant is not bounded reply metadata"));
        }
        Ok(())
    }
    pub(crate) fn capture_plan(&self, session: &str, parent: bool) -> io::Result<CapturePlan> {
        self.check()?;
        let key = SourceKey::new(session, &self.source)?;
        let mut plan = CapturePlan::new(key, CaptureGeneration::new(self.generation));
        plan.bind_contract(None)?;
        for (phase, row) in self.phases.iter().enumerate() {
            for (owner, bytes) in row.iter().copied().enumerate() {
                if bytes != 0 {plan.add_bytes(OWNERS[owner], bytes, 1, CapturePhases::one(PHASES[phase]))?;}
            }
        }
        if plan.peak_bytes() == 0 || plan.owner_limits()[CaptureOwner::SourcePixels as usize] < self.raw_bytes {
            return Err(invalid("login capture raw credit differs from source plan"));
        }
        if parent && self.raw_bytes != 0 {plan.add_bytes(CaptureOwner::Transport, self.raw_bytes, 1, CapturePhases::ALL)?;}
        Ok(plan)
    }
    pub(crate) fn matches(&self, other: &Self) -> bool {
        self.source == other.source && self.generation == other.generation
            && self.phases == other.phases && self.raw_bytes == other.raw_bytes
    }
}
pub(crate) fn raw_length(width: u32, height: u32) -> io::Result<usize> {
    let length = width.checked_mul(height).and_then(|n| n.checked_mul(4))
        .filter(|n| *n != 0 && *n as usize <= super::model::MAX_PIXELS)
        .ok_or_else(|| invalid("login helper pixel geometry exceeds bounded raw frame"))?;
    Ok(length as usize)
}
fn invalid(reason: &str) -> io::Error {io::Error::new(io::ErrorKind::InvalidData, reason)}
