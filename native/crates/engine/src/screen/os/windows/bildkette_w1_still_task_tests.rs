//! Task tests of merge W1 (plan 2026-11-14 „Bildkette“, request 5): a
//! still image on the GPU route is the capture's BGRA, read back once in
//! bands within the `Readback` budget the NV12 ring leaves, bit-identical
//! to the CPU route's readback. The device is this machine's (WARP on a
//! runner without GPU); without any D3D11 device that part returns early.
use std::sync::Arc;

use super::*;
use crate::screen::core::capture_admission::{CaptureGeneration, CapturePlan, SourceKey};
use crate::screen::core::capture_plan::{bgra_bytes, CapturePhases};
use crate::screen::core::frame_budget::MAX_WORKING_BYTES;
use crate::screen::core::frame_memory::{CapacityCeiling, MemoryPool};
use crate::screen::os::mf_encoder::video_convert::{nv12_len, slot_size};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DEFAULT,
};

/// A grant with the `Readback` budget of one native BGRA staging texture,
/// as `capture_slots.rs` plans it for an unscaled capture.
fn readback_grant(size: FrameSize) -> CaptureGrant {
    let name = "bildkette-w1-still";
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(MAX_WORKING_BYTES),
            MAX_WORKING_BYTES,
            name,
        )
        .unwrap();
    admission.bind_contract(None).unwrap();
    let mut plan = CapturePlan::new(
        SourceKey::new(name, "bildkette-fixture").unwrap(),
        CaptureGeneration::new(1),
    );
    plan.add_bgra(CaptureOwner::Readback, size, 1, CapturePhases::ALL)
        .unwrap();
    admission.authority().approve(plan).unwrap()
}

#[test]
fn bildkette_task_win_w1_still_image_bands_fit_beside_the_ring() {
    // Ring (two NV12 slots) and one band stay within the one BGRA staging
    // texture the plan admits (`Readback`), for every picture of 4 rows up.
    let sizes = [
        (1920, 1080),
        (3840, 2160),
        (1366, 768),
        (1001, 701),
        (640, 8),
        (64, 4),
    ];
    for (width, height) in sizes {
        let size = FrameSize::new(width, height);
        let rows = exact_rows(size);
        assert!((1..=64).contains(&rows), "{size:?}");
        let ring = 2 * nv12_len(slot_size(size));
        let band = bgra_bytes(FrameSize::new(width, rows)).unwrap();
        assert!(ring + band <= bgra_bytes(size).unwrap(), "{size:?}");
    }
    assert_eq!(bands(10, 4), vec![(0, 4), (4, 4), (8, 2)]);
    assert_eq!(bands(3, 64), vec![(0, 3)]);
    assert_eq!(bands(5, 0), bands(5, 1), "at least one row per band");
}

#[test]
fn bildkette_task_win_w1_still_image_is_bit_identical_to_the_cpu_route() {
    let Ok(devices) = crate::screen::os::d3d_device::create() else {
        return;
    };
    let size = FrameSize::new(67, 45);
    let mut state = 0x9e37_79b9u32;
    let data: Vec<u8> = (0..size.bgra_len())
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size.width,
        Height: size.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: data.as_ptr().cast(),
        SysMemPitch: size.width * 4,
        SysMemSlicePitch: 0,
    };
    let mut source = None;
    // SAFETY: `data` outlives the call and holds every row of the texture.
    unsafe {
        devices
            .device
            .CreateTexture2D(&desc, Some(&initial), Some(&mut source))
    }
    .expect("source texture");
    let source = source.expect("source texture");
    let grant = readback_grant(size);
    // The CPU route: the whole picture through one staging texture.
    let full = staging_texture(&devices, size, &grant).expect("staging");
    let mut cpu = vec![0u8; size.bgra_len()];
    copy_band(&devices, &source, &full.texture, size.width, 0, size.height);
    map_into(&devices, &full.texture, size, &mut cpu).expect("CPU route");
    drop(full);
    // The still image: bands of the rows the ring leaves room for, and of
    // band heights that do not divide the picture.
    for rows in [exact_rows(size), 7, 1] {
        let band = FrameSize::new(size.width, rows);
        let staging = staging_texture(&devices, band, &grant).expect("band");
        let mut exact = vec![0u8; size.bgra_len()];
        read_bands(&devices, &source, &staging.texture, size, rows, &mut exact)
            .expect("still image");
        assert_eq!(exact, cpu, "rows {rows}");
    }
    // Both are the capture's BGRA itself, opaque as the CPU route makes it.
    let mut expected = data;
    for pixel in expected.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    assert_eq!(cpu, expected);
}
