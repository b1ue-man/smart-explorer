//! Independent family surfaces composed over an opaque blank canvas. Never crop
//! a desktop screenshot: that would expose unrelated overlapping applications.
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};
use crate::remote_window::{CaptureBoundary, HostSession, NativeWindowId, WindowAdapter,
    WindowAuthority, WindowIdentity};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_backend::{ScreenSource, SourceFrame};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::session_state::SessionState;

pub(crate) type SharedHost = Arc<Mutex<HostSession<Box<dyn WindowAdapter>>>>;
pub(crate) type SharedAuthority = Arc<dyn WindowAuthority + Send + Sync>;
pub(crate) type OpenMember = Box<dyn FnMut(NativeWindowId) -> io::Result<Box<dyn ScreenSource>> + Send>;

struct Surface {
    source: Box<dyn ScreenSource>,
    image: Option<BgraImage>,
}

pub(crate) struct FamilyScreenSource {
    host: SharedHost,
    authority: SharedAuthority,
    identity: WindowIdentity,
    open: OpenMember,
    surfaces: HashMap<WindowIdentity, Surface>,
    last_boundary: Option<CaptureBoundary>,
    reference_boundary: Option<CaptureBoundary>,
    size: FrameSize,
    limits: FrameLimits,
    active: bool,
    reference_generation: u64,
}

fn lock(host: &SharedHost) -> io::Result<MutexGuard<'_, HostSession<Box<dyn WindowAdapter>>>> {
    host.lock().map_err(|_| io::Error::other("window_host_state_poisoned"))
}

impl FamilyScreenSource {
    /// `open` must open the platform's raw WGC/XComposite source in this same
    /// authorized host worker, with the existing capture indicator/security rules.
    pub(crate) fn new(host: SharedHost, authority: SharedAuthority, identity: WindowIdentity,
        open: OpenMember, limits: FrameLimits) -> io::Result<Self> {
        let boundary = lock(&host)?.boundary(&identity, authority.as_ref())?;
        let bounds = match boundary.bounds() {
            Some(bounds) => bounds,
            None => lock(&host)?.snapshot(authority.as_ref())?.members.iter()
                .find(|member| member.identity == identity).map(|member| member.geometry)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "original_window_gone"))?,
        };
        let size = FrameSize::new(bounds.width, bounds.height);
        // The compositor bounds its source allocation before wire negotiation.
        limits.check(size)?;
        Ok(Self { host, authority, identity, open, surfaces: HashMap::new(), last_boundary: None,
            reference_boundary: None, size, limits, active: true, reference_generation: 1 })
    }

    fn note_boundary(&mut self, boundary: &CaptureBoundary) -> io::Result<()> {
        if self.reference_boundary.as_ref() != Some(boundary) {
            self.reference_generation = self.reference_generation.checked_add(1)
                .ok_or_else(|| io::Error::other("window_capture_reference_generation_exhausted"))?;
            self.reference_boundary = Some(boundary.clone());
        }
        Ok(())
    }

    fn invalidate_reference(&mut self) -> io::Result<()> {
        // One invalidation per accepted reference. A repeated pause for the
        // same observed layout must not keep advancing the worker/input floor.
        if self.last_boundary.take().is_some() {
            self.reference_generation = self.reference_generation.checked_add(1)
                .ok_or_else(|| io::Error::other("window_capture_reference_generation_exhausted"))?;
        }
        Ok(())
    }

    fn compose(&self, boundary: &CaptureBoundary) -> io::Result<BgraImage> {
        let bounds = boundary.bounds().ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock,
            "window_family_has_no_visible_surface"))?;
        let mut image = BgraImage::new_with_limits(FrameSize::new(bounds.width, bounds.height), self.limits)?;
        let mut members: Vec<_> = boundary.members.iter().collect();
        members.sort_by_key(|member| member.z_order);
        for member in members {
            let surface = self.surfaces.get(&member.identity).and_then(|s| s.image.as_ref())
                .ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock, "window_surface_not_ready"))?;
            if surface.size() != FrameSize::new(member.geometry.width, member.geometry.height) {
                return Err(io::Error::new(io::ErrorKind::WouldBlock, "window_capture_geometry_changed"));
            }
            let left = usize::try_from(i64::from(member.geometry.x) - i64::from(bounds.x))
                .map_err(|_| io::Error::other("invalid_window_surface_offset"))?;
            let top = u32::try_from(i64::from(member.geometry.y) - i64::from(bounds.y))
                .map_err(|_| io::Error::other("invalid_window_surface_offset"))?;
            let bytes = (member.geometry.width as usize).checked_mul(4)
                .ok_or_else(|| io::Error::other("window_surface_size_overflow"))?;
            for y in 0..member.geometry.height {
                let start = left.checked_mul(4).ok_or_else(|| io::Error::other("window_surface_offset_overflow"))?;
                let row = image.row_mut(top + y).get_mut(start..start + bytes)
                    .ok_or_else(|| io::Error::other("window_surface_outside_canvas"))?;
                row.copy_from_slice(surface.view().row(y));
                for pixel in row.chunks_exact_mut(4) { pixel[3] = 255; }
            }
        }
        Ok(image)
    }
}

impl ScreenSource for FamilyScreenSource {
    fn size(&self) -> FrameSize { self.size }
    fn frame_limits(&self) -> FrameLimits { self.limits }
    /// A changed member/layout invalidates references even at the same size.
    fn reference_generation(&self) -> u64 { self.reference_generation }
    fn family_layout_revision(&self) -> Option<u64> {
        self.last_boundary.as_ref().map(|boundary| boundary.revision)
    }
    fn supports_frame_regions(&self) -> bool { true }
    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        if active != self.active { self.invalidate_reference()?; }
        self.active = false;
        let mut error = None;
        for surface in self.surfaces.values_mut() {
            if let Err(failure) = surface.source.set_capture_active(active) { error.get_or_insert(failure); }
        }
        if let Some(error) = error {
            for surface in self.surfaces.values_mut() { let _ = surface.source.set_capture_active(false); }
            return Err(error);
        }
        self.active = active;
        Ok(())
    }

    fn capture(&mut self, frame_id: u64, keyframe: bool) -> io::Result<SourceFrame> {
        if !self.active { return Ok(SourceFrame::Unchanged); }
        let boundary = lock(&self.host)?.boundary(&self.identity, self.authority.as_ref())?;
        // Publish the new reference epoch before every outcome, including a
        // paused/minimized source or a surface that has not delivered pixels yet.
        self.note_boundary(&boundary)?;
        if boundary.members.is_empty() {
            let snapshot = lock(&self.host)?.snapshot(self.authority.as_ref())?;
            let minimized = snapshot.members.iter().find(|m| m.identity == snapshot.root)
                .is_some_and(|m| m.minimized);
            return Ok(SourceFrame::Paused(if minimized { SessionState::WindowMinimized } else { SessionState::Starting }));
        }
        let bounds=boundary.bounds().ok_or_else(||io::Error::other("window_family_has_no_visible_surface"))?;
        self.limits.check(FrameSize::new(bounds.width,bounds.height))?;
        // Retained member pictures share one source-image slot; their sum must
        // be bounded before opening capture pools for newly observed members.
        let member_pixels=boundary.members.iter().try_fold(0u64,|total,member| {
            let size=FrameSize::new(member.geometry.width,member.geometry.height);
            self.limits.check(size)?;
            total.checked_add(size.pixels()).ok_or_else(||io::Error::other("window_family_size_overflow"))
        })?;
        if member_pixels>self.limits.max_pixels.min(self.limits.working_bytes/
            crate::screen::core::frame_budget::WORKING_IMAGES/4) {
            return Err(io::Error::other("window_family_capture_budget_exceeded"));
        }
        let changed_boundary = self.last_boundary.as_ref() != Some(&boundary);
        self.surfaces.retain(|identity, _| boundary.members.iter().any(|m| &m.identity == identity));
        let mut changed = changed_boundary;
        for member in &boundary.members {
            if !self.surfaces.contains_key(&member.identity) {
                let source = match (self.open)(member.binding.native) {
                    Ok(source) => source,
                    Err(error) => {
                        if !lock(&self.host)?.boundary_current(&boundary, self.authority.as_ref())? {
                            self.invalidate_reference()?;
                            return Ok(SourceFrame::Paused(SessionState::Starting));
                        }
                        return Err(error);
                    }
                };
                self.surfaces.insert(member.identity.clone(), Surface { source, image: None });
            }
            let surface = self.surfaces.get_mut(&member.identity)
                .ok_or_else(|| io::Error::other("window_surface_not_open"))?;
            let captured = surface.source.capture(frame_id, keyframe || changed_boundary || surface.image.is_none());
            let captured = match captured {
                Ok(captured) => captured,
                Err(error) => {
                    if !lock(&self.host)?.boundary_current(&boundary, self.authority.as_ref())? {
                        self.invalidate_reference()?;
                        return Ok(SourceFrame::Paused(SessionState::Starting));
                    }
                    return Err(error);
                }
            };
            match captured {
                SourceFrame::Frame(image) => { surface.image = Some(image); changed = true; }
                SourceFrame::Unchanged if surface.image.is_none() => return Ok(SourceFrame::Paused(SessionState::Starting)),
                SourceFrame::Unchanged => {}
                SourceFrame::Paused(state) => return Ok(SourceFrame::Paused(state)),
            }
        }
        if !lock(&self.host)?.boundary_current(&boundary, self.authority.as_ref())? {
            self.invalidate_reference()?;
            // The next full composition replaces the reference; stale surfaces
            // are never published after a handle/member/geometry change.
            return Ok(SourceFrame::Paused(SessionState::Starting));
        }
        if !changed && !keyframe { return Ok(SourceFrame::Unchanged); }
        let image = match self.compose(&boundary) {
            Ok(image) => image,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                self.invalidate_reference()?;
                return Ok(SourceFrame::Paused(SessionState::Starting));
            }
            Err(error) => return Err(error),
        };
        self.size = image.size();
        self.last_boundary = Some(boundary);
        Ok(SourceFrame::Frame(image))
    }
}
