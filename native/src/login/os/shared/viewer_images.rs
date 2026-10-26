//! Two actual raw images per local viewer; aliases never create free slots.
use crate::login::core::model::MAX_PIXELS;
use crate::screen::core::image_admission::{ImageAdmission, ImageOwnerLimits, ImageRole};
use std::{io, sync::Arc};
pub(crate) fn limits() -> io::Result<ImageOwnerLimits> {
    ImageOwnerLimits::new(2 * MAX_PIXELS as u64)?.with_role(
        ImageRole::ViewerInput,
        MAX_PIXELS as u64,
        2,
    )
}
pub(crate) fn admission() -> io::Result<Arc<ImageAdmission>> {
    crate::screen::os::image_memory::admit_images(limits()?)
}
pub(crate) fn can_capture(admission: &ImageAdmission) -> io::Result<bool> {
    Ok(admission.live_count(ImageRole::ViewerInput)? < 2)
}
