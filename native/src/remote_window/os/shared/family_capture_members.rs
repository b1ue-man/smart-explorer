//! Member admission cannot create a new root behind the compositor's back.
use super::*;
use crate::screen::core::capture_admission::{CaptureAuthority, SessionAdmission};
use crate::screen::core::capture_import::CaptureImportNeed;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct MemberAuthority {
    parent: Arc<SessionAdmission>,
    changed: Arc<AtomicBool>,
}

impl CaptureAuthority for MemberAuthority {
    fn approve_supplement(
        &self,
        image: &CaptureGrant,
        plan: CapturePlan,
    ) -> io::Result<CaptureGrant> {
        image.check_supplement(&plan)?;
        self.parent.authority().approve_supplement(image, plan)
    }
    fn approve(&self, _plan: CapturePlan) -> io::Result<CaptureGrant> {
        self.changed.store(true, Ordering::SeqCst);
        Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "family member requires coordinated capture readmission",
        ))
    }

    fn register_prepared_source(&self, source: SourceKey) -> io::Result<()> {
        self.parent.register_prepared_source(source)
    }

    fn unregister_prepared_source(&self, source: &SourceKey) {
        self.parent.authority().unregister_prepared_source(source);
    }

    fn own_prepared_source(
        &self,
        source: SourceKey,
    ) -> io::Result<crate::screen::core::capture_admission::PreparedSourceOwnerId> {
        self.parent.authority().own_prepared_source(source)
    }

    fn release_prepared_source(
        &self,
        source: &SourceKey,
        owner: crate::screen::core::capture_admission::PreparedSourceOwnerId,
    ) {
        self.parent
            .authority()
            .release_prepared_source(source, owner);
    }

    fn begin_import_connection(&self, source: &SourceKey) -> io::Result<u64> {
        self.parent.authority().begin_import_connection(source)
    }

    fn end_import_connection(&self, source: &SourceKey, generation: u64) {
        self.parent
            .authority()
            .end_import_connection(source, generation);
    }

    fn cancel_pending(&self, source: &SourceKey) {
        self.parent.cancel_pending_capture(source);
    }

    fn approve_import(
        &self,
        need: CaptureImportNeed,
    ) -> io::Result<crate::screen::core::capture_import::CaptureImportGrant> {
        self.parent.authority().approve_import(need)
    }
}

pub(super) fn member_admission(
    parent: &Arc<SessionAdmission>,
    output: &HostOutputPlan,
    changed: Arc<AtomicBool>,
) -> io::Result<Arc<SessionAdmission>> {
    let authority = Arc::new(MemberAuthority {
        parent: parent.clone(),
        changed,
    });
    Ok(Arc::new(SessionAdmission::from_verified_parent(
        parent.session(),
        parent.working_bytes(),
        output.contract().frame_contract(),
        authority,
        parent.clone(),
    )?))
}

pub(super) struct Surface {
    pub identity: WindowIdentity,
    pub source: Box<dyn ScreenSource>,
    pub image: Option<CapturedImage>,
    pub grant: CaptureGrant,
}

/// Vec storage is destroyed before the slice which admitted its allocation.
pub(super) struct Surfaces {
    pub boundary: CaptureBoundary,
    pub values: Vec<Surface>,
    _metadata: crate::screen::core::capture_admission::CaptureSlice,
}

impl Surfaces {
    pub fn activate(
        mut prepared: super::planning::PreparedSet,
        grant: &CaptureGrant,
    ) -> io::Result<Self> {
        let bytes = super::planning::surface_bytes(&prepared.boundary)?;
        let metadata = grant.claim(CaptureOwner::Metadata, bytes)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(prepared.members.len())
            .map_err(|_| io::Error::other("family surface container allocation failed"))?;
        if values.capacity() > prepared.members.len() {
            return Err(io::Error::other(
                "family surface container exceeded prior promise",
            ));
        }
        for member in prepared.members.drain(..) {
            let member_grant = grant.for_member(
                member.source.source_key(),
                member.source.description().generation,
            )?;
            let expected = member_grant.clone();
            let source = member
                .source
                .activate(member_grant)
                .map_err(|refusal| io::Error::other(refusal.reason))?;
            values.push(Surface {
                identity: member.identity,
                source,
                image: None,
                grant: expected,
            });
        }
        Ok(Self {
            boundary: prepared.boundary,
            values,
            _metadata: metadata,
        })
    }
}

impl FamilyScreenSource {
    pub(super) fn compose(&self, boundary: &CaptureBoundary) -> io::Result<CapturedImage> {
        let bounds = boundary.bounds().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "window_family_has_no_visible_surface",
            )
        })?;
        let grant = self
            .grant
            .as_ref()
            .ok_or_else(|| io::Error::other("family has no grant"))?;
        let mut image = CaptureImageBuilder::new(
            FrameSize::new(bounds.width, bounds.height),
            self.limits,
            grant,
            CaptureOwner::CompositionPixels,
        )?;
        let metadata = grant.claim(
            CaptureOwner::Metadata,
            (boundary.members.len() * std::mem::size_of::<usize>()) as u64,
        )?;
        let mut order = Vec::new();
        order
            .try_reserve_exact(boundary.members.len())
            .map_err(|_| io::Error::other("family sort allocation failed"))?;
        if order.capacity() > boundary.members.len() {
            return Err(io::Error::other(
                "family sort container exceeded prior promise",
            ));
        }
        order.extend(0..boundary.members.len());
        order.sort_unstable_by_key(|index| (boundary.members[*index].z_order, *index));
        let surfaces = &self
            .surfaces
            .as_ref()
            .ok_or_else(|| io::Error::other("family surfaces missing"))?
            .values;
        for index in &order {
            let member = &boundary.members[*index];
            let surface = surfaces
                .iter()
                .find(|surface| surface.identity == member.identity)
                .and_then(|surface| surface.image.as_ref())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::WouldBlock, "window_surface_not_ready")
                })?;
            if surface.size() != FrameSize::new(member.geometry.width, member.geometry.height) {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "window_capture_geometry_changed",
                ));
            }
            let left = usize::try_from(i64::from(member.geometry.x) - i64::from(bounds.x))
                .map_err(|_| io::Error::other("invalid_window_surface_offset"))?;
            let top = u32::try_from(i64::from(member.geometry.y) - i64::from(bounds.y))
                .map_err(|_| io::Error::other("invalid_window_surface_offset"))?;
            let start = left
                .checked_mul(4)
                .ok_or_else(|| io::Error::other("window_surface_offset_overflow"))?;
            let bytes = member.geometry.width as usize * 4;
            for y in 0..member.geometry.height {
                let row = image
                    .row_mut(top + y)
                    .and_then(|row| row.get_mut(start..start + bytes))
                    .ok_or_else(|| io::Error::other("window_surface_outside_canvas"))?;
                row.copy_from_slice(surface.view().row(y));
                for pixel in row.chunks_exact_mut(4) {
                    pixel[3] = 255;
                }
            }
        }
        drop(order);
        drop(metadata);
        Ok(image.freeze())
    }
}

#[cfg(test)]
mod supplement_tests {
    use super::*;
    use crate::screen::core::capture_plan::CapturePhases;
    #[test]
    fn batches_task_capture_integration_family_cursor_preserves_root_and_changed_flag() {
        let parent = crate::screen::core::capture_test_support::session(None);
        let source = SourceKey::new(parent.session(), "cursor-member").unwrap();
        let mut member = CapturePlan::new(source.clone(), CaptureGeneration::new(4));
        member
            .add_bytes(CaptureOwner::SourcePixels, 8, 2, CapturePhases::ALL)
            .unwrap();
        let mut plan = CapturePlan::new(
            SourceKey::new(parent.session(), "family").unwrap(),
            CaptureGeneration::new(2),
        );
        plan.add_member(member).unwrap();
        let root = parent.authority().approve(plan).unwrap();
        let image = root.for_member(&source, CaptureGeneration::new(4)).unwrap();
        let changed = Arc::new(AtomicBool::new(false));
        let output = HostOutputPlan::new(
            crate::screen::core::host_codec_adapters::HostContract::Legacy,
            FrameSize::new(2, 1),
            crate::screen::core::host_codec_adapters::HostCodecPlan::Tiles,
        )
        .unwrap();
        let context = member_admission(&parent, &output, changed.clone()).unwrap();
        for _ in 0..2 {
            let mut cursor = CapturePlan::new(source.clone(), CaptureGeneration::new(4));
            cursor
                .add_bytes(CaptureOwner::Readback, 64, 1, CapturePhases::ALL)
                .unwrap();
            cursor
                .add_bytes(CaptureOwner::Metadata, 8, 1, CapturePhases::ALL)
                .unwrap();
            let cursor = context
                .authority()
                .approve_supplement(&image, cursor)
                .unwrap();
            let parsed = cursor.claim(CaptureOwner::Metadata, 8).unwrap();
            assert!(!changed.load(Ordering::SeqCst));
            assert!(!root.is_retired().unwrap());
            assert!(image.claim(CaptureOwner::SourcePixels, 8).is_ok());
            drop(cursor);
            drop(parsed);
        }
        let mut geometry = CapturePlan::new(source, CaptureGeneration::new(5));
        geometry
            .add_bytes(CaptureOwner::SourcePixels, 16, 2, CapturePhases::ALL)
            .unwrap();
        assert_eq!(
            context.authority().approve(geometry).err().unwrap().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(
            changed.load(Ordering::SeqCst),
            "real geometry changes still coordinate family readmission"
        );
    }
}
