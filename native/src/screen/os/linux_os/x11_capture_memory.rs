//! Metadata-only ownership plan shared by prepare, resize and parser gating.
use std::io;
use crate::screen::core::capture_admission::{CaptureGeneration, CaptureGrant, SourceKey};
use crate::screen::core::capture_plan::{CaptureOwner, CapturePhases, CapturePlan, HostPhase};
use crate::screen::core::host_codec_adapters::{HostInputPixels, HostOutputPlan};
use crate::screen::core::pixel_convert::FrameSize;

pub(super) fn plan(source: &SourceKey, generation: CaptureGeneration, size: FrameSize,
    shm: bool, pixmap_bytes: u64, output: Option<&HostOutputPlan>) -> io::Result<CapturePlan> {
    let mut plan = CapturePlan::new(source.clone(), generation);
    // Latest/cache and every borrowed result alias one storage. The second
    // slot is the next producer; additional held results exhaust these slots.
    plan.add_bgra(CaptureOwner::SourcePixels, size, 2, CapturePhases::ALL)?;
    let bytes = crate::screen::core::capture_plan::bgra_bytes(size)?;
    plan.add_bytes(if shm {CaptureOwner::NativeSurface} else {CaptureOwner::Readback},
        if shm {bytes} else {(bytes + 32).max(64)}, 1,
        if shm {CapturePhases::ALL} else {CapturePhases::one(HostPhase::Capture)})?;
    if pixmap_bytes > 0 {
        plan.add_bytes(CaptureOwner::NativeSurface, pixmap_bytes, 1,
            CapturePhases::one(HostPhase::Capture))?;
    }
    if let Some(output) = output {
        output.append_to(&mut plan, if output.size() == size {HostInputPixels::BorrowedSource}
            else {HostInputPixels::SeparateImage})?;
    }
    Ok(plan)
}

pub(super) fn verify(grant: &CaptureGrant, expected: &CapturePlan) -> io::Result<()> {
    grant.check_identity(expected.source(), expected.generation())?;
    if grant.plan().peak_bytes() < expected.peak_bytes()
        || grant.plan().owner_limits().iter().zip(expected.owner_limits()).any(|(have, need)| *have < need) {
        return Err(io::Error::other("x11_capture_grant_too_small"));
    }
    Ok(())
}


impl super::X11Source {
    pub(super) fn pixmap_geometry(&self, window: x11rb::protocol::xproto::Window)
        -> io::Result<(u16, FrameSize, u64)>
    {
        use x11rb::protocol::xproto::ConnectionExt as _;
        let geo = self.conn.get_geometry(window).map_err(super::lost)?
            .reply().map_err(|error| super::reply_error(error, true))?;
        let size = FrameSize::new(u32::from(geo.width), u32::from(geo.height));
        let border = u64::from(geo.border_width) * 2;
        let bytes = (u64::from(geo.width) + border)
            .checked_mul(u64::from(geo.height) + border)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| super::failed("pixmap size overflow"))?;
        Ok((geo.border_width, size, bytes))
    }

    pub(in super::super) fn plan(&self, output: Option<&HostOutputPlan>) -> io::Result<CapturePlan> {
        let pixmap = match self.target {
            super::Target::Window(window) if self.compositor => self.pixmap_geometry(window)?.2,
            _ => 0,
        };
        plan(&self.source_key, self.generation, self.size, self.shm, pixmap, output)
    }

    pub(super) fn admit_geometry(&mut self, area: super::super::x11_input::Area) -> io::Result<()> {
        let pixmap = match self.target {
            super::Target::Window(window) if self.compositor => self.pixmap_geometry(window)?.2,
            _ => 0,
        };
        if self.geometry == Some(area) && self.pixmap_bytes == pixmap {
            return Ok(());
        }
        let size = FrameSize::new(area.width as u32, area.height as u32);
        crate::screen::core::frame_budget::FrameLimits::RESOURCE.check(size)?;
        let generation = self.generation.next()?;
        let next = plan(&self.source_key, generation, size, self.shm, pixmap, self.output.as_ref())?;
        if let Some(segment) = self.segment.take() {
            segment.release(&self.conn);
        }
        self.last_image = None;
        self.last_hash = None;
        if let Some(grant) = &self.grant {
            if !grant.is_member() {
                grant.retire()?;
            }
        }
        let grant = self.admission.authority().approve(next)?;
        self.grant = Some(grant);
        self.generation = generation;
        self.pixmap_bytes = pixmap;
        self.note_geometry(area)
    }
}
