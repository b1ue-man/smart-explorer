//! The route of one captured picture (plan 2026-11-14 bildkette B-17,
//! package BK2a, rank of 4.10a; split from `capture_readback.rs` by plan
//! 2026-11-17 P9): into the NV12 ring on the capture's GPU while an encoder
//! of this process takes the capture's textures (`gpu_picture`), else as CPU
//! pixels, and why (`CpuReason`).
//!
//! P9: a Desktop Duplication picture whose pointer the host blends no
//! longer leaves the GPU for it; the pointer is drawn into the ring slot as
//! a small patch (`capture_pointer_gpu.rs`). The reason of the last picture
//! that went as CPU pixels is kept for the chain facts of the host process
//! (`chain_facts.rs`, `last_cpu_reason`), so `se screen sessions --json` and
//! the bench name it beside `convert cpu`.
use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use super::pointer_gpu::PointerAt;
use super::Readback;
use crate::screen::core::capture_admission::CaptureOwner;
use crate::screen::core::capture_picture::GpuPicture;
use crate::screen::core::pixel_convert::BgraRotation;
use crate::screen::os::capture_slots::{CaptureBudget, CaptureKind};
use crate::screen::os::d3d_device::DeviceSet;
use crate::screen::os::mf_capture_media::RingPicture;
use crate::screen::os::mf_encoder::d3d_input;
use crate::screen::os::mf_encoder::video_convert::Ring;

/// Why a picture is read back to the CPU instead of staying on the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::screen::os) enum CpuReason {
    /// No encoder of this process takes textures.
    NotArmed,
    /// The texture encoder runs on another device than this capture.
    OtherDevice,
    /// The GPU path failed earlier in this session.
    Disabled,
    /// The capture device has no video support.
    NoVideo,
    /// Desktop Duplication of a rotated output (the CPU rotates).
    Rotated,
    /// The readback scales to the capture limits.
    Scaled,
    /// The host blends the pointer into Desktop Duplication pictures on the
    /// CPU (P9: only after the GPU blend failed in this capture).
    PointerBlended,
    /// A still image asked for this picture's exact BGRA (merge W1): one
    /// banded readback, the NV12 ring stays for the next pictures.
    Exact,
}

impl CpuReason {
    /// The reasons in the order of their code in `LAST`.
    const ALL: [Self; 8] = [
        Self::NotArmed,
        Self::OtherDevice,
        Self::Disabled,
        Self::NoVideo,
        Self::Rotated,
        Self::Scaled,
        Self::PointerBlended,
        Self::Exact,
    ];
}

/// What the capture side knows when a picture arrives.
#[derive(Clone, Copy, Debug)]
pub(in crate::screen::os) struct CaptureFacts {
    pub armed: Option<usize>,
    pub device: usize,
    pub disabled: bool,
    pub video: bool,
    pub rotated: bool,
    pub scaled: bool,
    pub duplication: bool,
    /// The viewer draws the pointer itself (P6 overlay).
    pub overlay: bool,
    /// P9: the pointer can be drawn into the picture on the GPU.
    pub gpu_pointer: bool,
}

/// Rank of 4.10a, conversion: on the GPU of the capture when the encoder
/// takes textures, else on the CPU. P9: a pointer the host blends keeps the
/// GPU route while it can be drawn there.
pub(in crate::screen::os) fn capture_route(facts: CaptureFacts) -> Result<(), CpuReason> {
    let cpu_pointer = facts.duplication && !facts.overlay && !facts.gpu_pointer;
    match facts.armed {
        _ if facts.disabled => Err(CpuReason::Disabled),
        None => Err(CpuReason::NotArmed),
        Some(device) if device != facts.device => Err(CpuReason::OtherDevice),
        Some(_) if !facts.video => Err(CpuReason::NoVideo),
        Some(_) if facts.rotated => Err(CpuReason::Rotated),
        Some(_) if facts.scaled => Err(CpuReason::Scaled),
        Some(_) if cpu_pointer => Err(CpuReason::PointerBlended),
        Some(_) => Ok(()),
    }
}

/// The route of the last picture of this process: 0 none yet, 1 the GPU,
/// else 2 + the index of its `CpuReason`.
static LAST: AtomicU8 = AtomicU8::new(0);

/// Notes the route of a picture; a still image's exact readback is no
/// route of the stream and is not noted.
pub(in crate::screen::os) fn note_route(route: Result<(), CpuReason>) {
    let code = match route {
        Ok(()) => 1,
        Err(CpuReason::Exact) => return,
        Err(reason) => CpuReason::ALL
            .iter()
            .position(|known| *known == reason)
            .map_or(0, |index| index as u8 + 2),
    };
    LAST.store(code, Ordering::Relaxed);
}

/// Why the last picture of this process went as CPU pixels; `None` when
/// it stayed on the GPU or no picture was routed yet.
pub(in crate::screen::os) fn last_cpu_reason() -> Option<CpuReason> {
    let code = LAST.load(Ordering::Relaxed);
    CpuReason::ALL
        .get(usize::from(code.checked_sub(2)?))
        .copied()
}

impl Readback {
    /// B-17: the picture goes into the NV12 ring on this device and the
    /// pipeline gets the GPU picture of its slot; else why the CPU reads it.
    /// P9: `pointer` is drawn into the slot.
    pub(super) fn gpu_picture(
        &mut self,
        devices: &DeviceSet,
        source: &ID3D11Texture2D,
        budget: &CaptureBudget,
        pointer: Option<PointerAt<'_>>,
    ) -> io::Result<Result<GpuPicture, CpuReason>> {
        let geometry = budget.geometry();
        let raw = devices.device.as_raw() as usize;
        let gpu_pointer = self.patch.usable();
        let route = d3d_input::route_capture(devices, |armed, disabled, video| CaptureFacts {
            armed,
            device: raw,
            disabled,
            video,
            rotated: matches!(geometry.kind, CaptureKind::Dxgi(r) if r != BgraRotation::Identity),
            scaled: geometry.delivered != geometry.native,
            duplication: matches!(geometry.kind, CaptureKind::Dxgi(_)),
            overlay: crate::screen::core::cursor_overlay::worker().separated(),
            gpu_pointer,
        });
        if let Err(reason) = route {
            return Ok(Err(reason));
        }
        let grant = budget.grant()?;
        let identity = (grant.source().clone(), grant.generation());
        let staging = &mut self.staging;
        let found = d3d_input::ring_for(
            |ring| {
                ring.device == raw
                    && ring.source == identity.0
                    && ring.generation == identity.1
                    && ring.native == geometry.native
            },
            || *staging = None,
            || {
                Ring::create(
                    &devices.device,
                    &devices.context,
                    identity.clone(),
                    geometry.native,
                    |bytes| grant.claim(CaptureOwner::Readback, bytes),
                )
            },
        );
        let (ring, tag) = match found {
            Ok(found) => found,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Err(error),
            Err(error) => {
                d3d_input::disable(&format!("NV12 ring: {error}"));
                d3d_input::release_ring();
                return Ok(Err(CpuReason::Disabled));
            }
        };
        let patch = match pointer {
            Some(at) => match self
                .patch
                .prepare(devices, source, geometry.native, at, grant)
            {
                Ok(patch) => patch,
                Err(error) => return Ok(Err(self.pointer_failed(ring, &error))),
            },
            None => None,
        };
        let written = ring.write(source, tag, patch.as_ref().map(|(t, at)| (t, *at)));
        match written {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Ok(Err(self.pointer_failed(ring, &error))),
            Err(error) => {
                d3d_input::disable(&format!("video processor: {error}"));
                drop(ring);
                d3d_input::release_ring();
                return Ok(Err(CpuReason::Disabled));
            }
        }
        // Admitted by the capture's grant: the ring slots it names were
        // claimed from it (`Readback`), no system memory is needed.
        let surface = RingPicture {
            ring: Arc::downgrade(&ring),
            tag,
            size: ring.size,
        };
        let picture = GpuPicture::new(
            Arc::new(surface),
            geometry.delivered,
            geometry.limits,
            grant,
        )?;
        Ok(Ok(picture))
    }

    /// The pointer cannot be drawn on the GPU: this picture and the next
    /// ones go the CPU route, which blends it; the ring gives its budget to
    /// the CPU staging texture.
    fn pointer_failed(&mut self, ring: Arc<Ring>, error: &io::Error) -> CpuReason {
        self.patch.fail(error);
        drop(ring);
        d3d_input::release_ring();
        CpuReason::PointerBlended
    }
}
