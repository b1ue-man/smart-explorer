//! Platform facts enter the portable exact-image ledger at this one boundary.
use std::io;
use std::sync::Arc;
use crate::screen::core::frame_memory::{self, CapacityCeiling};
use crate::screen::core::image_admission::{self, ImageAdmission, ImageOwnerLimits, LocalImagePolicy};

pub(crate) fn local_image_policy() -> LocalImagePolicy {
    let facts = super::session_memory::facts();
    LocalImagePolicy {facts, ceiling: super::session_memory::capacity_ceiling(facts)}
}
pub(crate) fn admit_images(limits: ImageOwnerLimits) -> io::Result<Arc<ImageAdmission>> {
    image_admission::admit_images(local_image_policy(), limits)
}
/// Android calls this before IPC Open. Every existing pool access path then
/// respects the observed envelope, including active capture/session controllers.
pub(crate) fn observe_capacity_ceiling(ceiling: CapacityCeiling) -> io::Result<()> {
    frame_memory::observe_capacity_ceiling(ceiling)?;
    super::session_memory::observe_platform_ceiling(ceiling);
    Ok(())
}
