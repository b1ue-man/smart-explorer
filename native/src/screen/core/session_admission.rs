//! Demand and private legacy limits, independent of platform observations.
use std::io;
use super::{FrameCapabilities,FrameLimits,MAX_WORKING_BYTES};
use crate::screen::core::{frame_size_caps::PacketBudget,pixel_convert::FrameSize,
    wire::invalid,wire_hello::{ScreenClientHello,SessionMode}};

/// Desktop promotes in place; Android opens its full session independently.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(crate) enum PreviewAdmission {FullContract,Geometric}

/// A preview reserves its bounded image transaction, never its whole process.
/// Include the old sender's whole-tile record so the same request works with v1.
pub(crate) fn requested_bytes(hello:&ScreenClientHello,offered:u64)->io::Result<u64> {
    if hello.mode!=SessionMode::Preview {return Ok(offered);}
    let size=hello.display_size.unwrap_or(FrameSize::new(320,320));
    if size.width==0 || size.height==0 {return Err(invalid("empty screen preview geometry"));}
    let limits=FrameLimits {max_pixels:size.pixels().min(FrameLimits::LEGACY.max_pixels),..FrameLimits::LEGACY};
    let (_,required)=PacketBudget::legacy_memory(limits)?;
    Ok(offered.min(required))
}

/// v1 stays v1 on the wire. Only the actual local lease and renderer narrow
/// allocation rights. A complete legacy lease retains the existing v1 class.
pub(crate) fn legacy_limits(working:u64,texture:Option<FrameSize>)->io::Result<FrameLimits> {
    let mut limits=FrameLimits {working_bytes:working.min(MAX_WORKING_BYTES),..FrameLimits::LEGACY};
    if let Some(texture)=texture {
        limits.max_size=FrameSize::new(limits.max_size.width.min(texture.width),limits.max_size.height.min(texture.height));
    }
    limits.max_pixels=limits.max_pixels.min(limits.max_size.pixels());
    limits.validate()?;
    if limits.working_bytes==MAX_WORKING_BYTES {return Ok(limits);}
    let mut low=0;
    let mut high=limits.max_pixels.min(limits.working_bytes/24/4);
    while low<high {
        let pixels=low+(high-low).div_ceil(2);
        let candidate=FrameLimits {max_pixels:pixels,..limits};
        if PacketBudget::legacy_memory(candidate)?.1<=limits.working_bytes {low=pixels;} else {high=pixels-1;}
    }
    if low==0 {return Err(invalid("insufficient memory for one legacy frame transaction"));}
    limits.max_pixels=low;
    PacketBudget::for_receive(None,Some(limits))?;
    Ok(limits)
}

pub(crate) fn preview_capabilities(mut caps:FrameCapabilities,hello:&ScreenClientHello)->io::Result<FrameCapabilities> {
    if hello.mode==SessionMode::Preview {
        let size=hello.display_size.unwrap_or(FrameSize::new(320,320));
        caps.frames.max_size=FrameSize::new(caps.frames.max_size.width.min(size.width),caps.frames.max_size.height.min(size.height));
        caps.frames.max_pixels=caps.frames.max_pixels.min(caps.frames.max_size.pixels());
    }
    caps.with_working_budget(caps.frames.working_bytes)
}

#[cfg(test)]
#[path="batches_session_budget_compatibility_task_tests.rs"]
mod tests;
