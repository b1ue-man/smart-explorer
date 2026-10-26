//! Renderer facts bound both the advertised frame offer and every decoder allocation.
use eframe::egui;
use crate::screen::core::frame_budget::{FrameCapabilities,FrameLimits};
use crate::screen::core::pixel_convert::FrameSize;

pub(crate) fn capabilities(ctx:&egui::Context)->FrameCapabilities {
    let canvas=ctx.input(|input|input.max_texture_side).max(1).min(u32::MAX as usize) as u32;
    let side=super::screen_wgpu_yuv::texture_limit().map_or(canvas,|gpu|gpu.min(canvas));
    FrameCapabilities::resources(FrameSize::new(side,side))
}
pub(crate) fn check(ctx:&egui::Context,size:FrameSize)->std::io::Result<usize> {
    capabilities(ctx).frames.check(size)
}
pub(crate) fn checked_target(ctx:&egui::Context,target:[usize;2])->Option<[usize;2]> {
    let width=u32::try_from(target[0]).ok()?;let height=u32::try_from(target[1]).ok()?;
    let size=capabilities(ctx).frames.fit(FrameSize::new(width,height));
    FrameLimits::RESOURCE.check(size).ok()?;
    Some([size.width as usize,size.height as usize])
}
