//! Exact Windows capture ownership. Native surfaces count their requested
//! pixels; driver padding and private queues are not claimed as product bytes.
use crate::screen::core::capture_admission::{
    CaptureGeneration, CaptureGrant, CaptureSlice, SessionAdmission, SourceKey,
};
use crate::screen::core::capture_plan::{
    bgra_bytes, CaptureOwner as O, CapturePhases, CapturePlan, HostPhase,
};
use crate::screen::core::capture_source_contract::{HostInputPixels, HostOutputPlan};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::pixel_convert::{BgraRotation, FrameSize};
use std::io;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CaptureKind {
    Wgc,
    Dxgi(BgraRotation),
}
impl CaptureKind {
    fn surfaces(self) -> u64 {
        if self == Self::Wgc {
            2
        } else {
            1
        }
    }
    fn rotated(self) -> bool {
        matches!(self, Self::Dxgi(r) if r != BgraRotation::Identity)
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct CaptureGeometry {
    pub native: FrameSize,
    pub delivered: FrameSize,
    pub limits: FrameLimits,
    pub kind: CaptureKind,
}
impl CaptureGeometry {
    pub(super) fn staging(self) -> FrameSize {
        if self.delivered == self.native {
            self.native
        } else {
            FrameSize::new(self.native.width, self.native.height.min(64))
        }
    }
    pub(super) fn native_displayed(self) -> FrameSize {
        match self.kind {
            CaptureKind::Dxgi(rotation) => rotation.size(self.native),
            _ => self.native,
        }
    }
    pub(super) fn displayed(self) -> FrameSize {
        match self.kind {
            CaptureKind::Dxgi(rotation) => rotation.size(self.delivered),
            _ => self.delivered,
        }
    }
    pub(super) fn plan(
        self,
        key: SourceKey,
        generation: CaptureGeneration,
        output: &HostOutputPlan,
    ) -> io::Result<CapturePlan> {
        self.limits.validate_capture()?;
        self.limits.check(self.delivered)?;
        let mut plan = CapturePlan::new(key, generation);
        plan.bind_contract(output.contract().frame_contract())?;
        plan.add_bgra(
            O::NativeSurface,
            self.native,
            self.kind.surfaces(),
            CapturePhases::ALL,
        )?;
        plan.add_bgra(O::Readback, self.staging(), 1, CapturePhases::ALL)?;
        // Cache/result aliases share storage. Two distinct old/new images may
        // coexist; rotation has one additional unrotated temporary image.
        plan.add_bgra(
            O::SourcePixels,
            self.delivered,
            2 + u64::from(self.kind.rotated()),
            CapturePhases::ALL,
        )?;
        if self.delivered != self.native {
            plan.add_bgra(
                O::Readback,
                self.staging(),
                1,
                CapturePhases::one(HostPhase::Capture),
            )?;
            let sums = u64::from(self.delivered.width)
                .checked_mul(std::mem::size_of::<[u64; 4]>() as u64)
                .ok_or_else(|| io::Error::other("capture filter storage overflow"))?;
            plan.add_bytes(
                O::FilterScratch,
                sums,
                1,
                CapturePhases::one(HostPhase::Capture),
            )?;
        }
        if matches!(self.kind, CaptureKind::Dxgi(BgraRotation::Identity)) {
            let rects = u64::from(self.native.width.div_ceil(64))
                * u64::from(self.native.height.div_ceil(64))
                * 2;
            let native_moves = rects
                .checked_mul(std::mem::size_of::<
                    windows::Win32::Graphics::Dxgi::DXGI_OUTDUPL_MOVE_RECT,
                >() as u64)
                .ok_or_else(|| io::Error::other("capture move storage overflow"))?;
            let native_dirty = rects
                .checked_mul(std::mem::size_of::<windows::Win32::Foundation::RECT>() as u64)
                .ok_or_else(|| io::Error::other("capture dirty storage overflow"))?;
            let region = rects
                .checked_mul(
                    std::mem::size_of::<crate::screen::core::frame_regions::MoveRect>()
                        .max(std::mem::size_of::<crate::screen::core::pixel_convert::Rect>())
                        as u64,
                )
                .ok_or_else(|| io::Error::other("capture region storage overflow"))?;
            plan.add_bytes(
                O::Metadata,
                native_moves,
                1,
                CapturePhases::one(HostPhase::Capture),
            )?;
            plan.add_bytes(
                O::Metadata,
                native_dirty,
                1,
                CapturePhases::one(HostPhase::Capture),
            )?;
            // At most rects validated records per image; old and new immutable
            // images may each retain a distinct region allocation.
            plan.add_bytes(O::Metadata, region, 2, CapturePhases::ALL)?;
        }
        output.append_to(
            &mut plan,
            if output.size() == self.displayed() {
                HostInputPixels::BorrowedSource
            } else {
                HostInputPixels::SeparateImage
            },
        )?;
        Ok(plan)
    }
}

pub(super) struct CaptureBudget {
    admission: Arc<SessionAdmission>,
    key: SourceKey,
    generation: CaptureGeneration,
    output: HostOutputPlan,
    geometry: CaptureGeometry,
    grant: Option<CaptureGrant>,
}
impl CaptureBudget {
    pub(super) fn activate(
        admission: Arc<SessionAdmission>,
        key: SourceKey,
        geometry: CaptureGeometry,
        output: HostOutputPlan,
        grant: CaptureGrant,
    ) -> io::Result<Self> {
        let generation = grant.generation();
        grant.check_identity(&key, generation)?;
        let expected = geometry.plan(key.clone(), generation, &output)?;
        if grant.plan().peak_bytes() != expected.peak_bytes()
            || grant.plan().owner_limits() != expected.owner_limits()
            || grant.plan().contract_binding() != expected.contract_binding()
            || grant.plan().phase_table() != expected.phase_table()
            || grant.plan().concurrency_cases() != expected.concurrency_cases()
            || grant.plan().requirements().len() != expected.requirements().len()
            || grant
                .plan()
                .requirements()
                .iter()
                .zip(expected.requirements())
                .any(|(actual, wanted)| {
                    actual.source() != wanted.source()
                        || actual.generation() != wanted.generation()
                        || actual.owner() != wanted.owner()
                        || actual.bytes_each() != wanted.bytes_each()
                        || actual.count() != wanted.count()
                        || actual.phases() != wanted.phases()
                })
        {
            return Err(io::Error::other(
                "Windows capture grant does not match prepared ownership",
            ));
        }
        Ok(Self {
            admission,
            key,
            generation,
            output,
            geometry,
            grant: Some(grant),
        })
    }
    pub(super) fn grant(&self) -> io::Result<&CaptureGrant> {
        self.grant
            .as_ref()
            .ok_or_else(|| io::Error::other("Windows capture awaits an admitted generation"))
    }
    pub(super) fn geometry(&self) -> CaptureGeometry {
        self.geometry
    }
    pub(super) fn generation(&self) -> u64 {
        self.generation.value()
    }
    /// Caller has closed the old native resources and released its own cache
    /// and staging first. External immutable results keep their old grant.
    pub(super) fn replace(&mut self, geometry: CaptureGeometry) -> io::Result<()> {
        if let Some(grant) = &self.grant {
            if !grant.is_member() {
                grant.retire()?;
            }
        }
        drop(self.grant.take());
        self.generation = self.generation.next()?;
        let plan = geometry.plan(self.key.clone(), self.generation, &self.output)?;
        let grant = self.admission.authority().approve(plan)?;
        self.geometry = geometry;
        self.grant = Some(grant);
        Ok(())
    }
    pub(super) fn suspend(&mut self) {
        if let Some(grant) = self.grant.take() {
            if !grant.is_member() {
                if let Err(error) = grant.retire() {
                    eprintln!("Windows capture retirement: {error}");
                }
            }
        }
    }
    pub(super) fn claim_native(&self) -> io::Result<CaptureSlice> {
        self.grant()?.claim(O::NativeSurface, self.native_bytes()?)
    }
    fn native_bytes(&self) -> io::Result<u64> {
        bgra_bytes(self.geometry.native)?
            .checked_mul(self.geometry.kind.surfaces())
            .ok_or_else(|| io::Error::other("Windows native capture size overflow"))
    }
}

impl Drop for CaptureBudget {
    fn drop(&mut self) {
        self.suspend();
    }
}

#[cfg(test)]
#[path = "batches_capture_task_tests.rs"]
pub(super) mod batches_capture_task_tests;
