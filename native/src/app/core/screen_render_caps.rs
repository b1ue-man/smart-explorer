//! Renderer facts bound both the advertised frame offer and every decoder allocation.
use eframe::egui;
use crate::screen::core::frame_budget::{FrameCapabilities,FrameLimits};
use crate::screen::core::pixel_convert::FrameSize;

pub(crate) fn capabilities(ctx:&egui::Context)->FrameCapabilities {
    let canvas=ctx.input(|input|input.max_texture_side).max(1).min(u32::MAX as usize) as u32;
    let side=super::screen_render_owner::get(ctx).and_then(|owner|owner.texture_limit()).unwrap_or(canvas);
    FrameCapabilities::resources(FrameSize::new(side,side))
}
pub(crate) fn admit(ctx:&egui::Context)->std::io::Result<(FrameCapabilities,crate::screen::core::frame_memory::MemoryLease)> {
    if !super::screen_render_owner::get(ctx).is_some_and(|owner|owner.available()) {return Err(std::io::Error::other("screen renderer unavailable"));}
    let lease=crate::screen::os::session_memory::reserve(crate::screen::core::frame_budget::MAX_SESSION_WORKING_BYTES)?;
    let caps=FrameCapabilities::admitted(capabilities(ctx).texture_size,lease.bytes())?;
    Ok((caps,lease))
}
pub(crate) fn check(ctx:&egui::Context,size:FrameSize)->std::io::Result<usize> {
    // Admission is retained by StreamLink; rendering checks texture and absolute
    // image bounds without sampling memory again after its own buffers exist.
    let texture=capabilities(ctx).texture_size;
    FrameLimits {max_size:texture,..FrameLimits::RESOURCE}.check(size)
}
pub(crate) fn checked_target(ctx:&egui::Context,target:[usize;2])->Option<[usize;2]> {
    let width=u32::try_from(target[0]).ok()?;let height=u32::try_from(target[1]).ok()?;
    let size=FrameLimits {max_size:capabilities(ctx).texture_size,..FrameLimits::RESOURCE}.fit(FrameSize::new(width,height));
    FrameLimits::RESOURCE.check(size).ok()?;
    Some([size.width as usize,size.height as usize])
}

pub(crate) fn bind_texture(texture:&mut super::screen_texture_cpu::CpuTexture,session:&super::screen_session::ViewerSession,memory:crate::screen::core::frame_memory::MemoryHold) {
    let memory=memory.or_else(||session.shared.memory.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone());
    let limits=session.info().hello.as_ref().and_then(|hello|hello.frame_contract).map(|contract|contract.limits);
    let limits=limits.or_else(||memory.as_ref().map(|memory|FrameLimits {working_bytes:memory.bytes().min(crate::screen::core::frame_budget::MAX_WORKING_BYTES),..FrameLimits::LEGACY}));
    texture.bind_memory(memory,limits);
}

/// Only a concrete retirement/worker/decoder event wakes blocked admission.
/// This function runs on the decoder opener thread, never the GUI thread.
pub(crate) fn admit_wait(ctx:&egui::Context,shared:&super::screen_session::Shared)->std::io::Result<(FrameCapabilities,crate::screen::core::frame_memory::MemoryLease)> {
    loop {
        let epoch=super::screen_render_owner::event_epoch();
        if shared.presentation.lock().unwrap_or_else(std::sync::PoisonError::into_inner).closed {return Err(std::io::Error::new(std::io::ErrorKind::Interrupted,"screen session cancelled"));}
        match admit(ctx) {
            Ok(value)=>return Ok(value),
            Err(error)=>{
                let Some(owner)=super::screen_render_owner::get(ctx).filter(|owner|owner.available()&&owner.admission_pending()) else{return Err(error);};
                ctx.request_repaint();drop(owner);super::screen_render_owner::wait_event(epoch);
            }
        }
    }
}
