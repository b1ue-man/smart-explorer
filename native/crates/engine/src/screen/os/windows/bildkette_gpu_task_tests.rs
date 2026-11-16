//! Plan 2026-11-14 bildkette, package BK2a (B-17, B-19, B-20, B-21): choice
//! between GPU and CPU path, mapping of the encoder settings, the GPU
//! picture of a ring slot (BK3b, replaces the placeholder tags), the
//! parallel CPU conversion against the reference, and the fallback without a
//! usable GPU. The test with a real video processor is
//! ignored (Suite bildkette-task).
use super::d3d_input::{encoder_route, EncoderFacts, SystemReason};
use super::video_convert::{
    nv12_len, nv12_texture, slot_size, with_mapped, VideoConverter, RGB_FULL_RANGE,
    YCBCR_BT709_LIMITED,
};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::pixel_convert::{
    bgra_to_i420_with_limits, BgraImage, FrameSize, YuvMatrix,
};
use crate::screen::os::capture_readback::{capture_route, CaptureFacts, CpuReason};
use crate::screen::os::mf_capture_media::{
    bgra_to_nv12, newer, next_tag, pick, ring_picture, RingPicture, TAG_COUNT,
};
use crate::screen::os::mf_media::{
    bitrate_limit, bounded_bitrate, gop_candidates, level_max_bitrate, ltr_control,
    one_frame_buffer, SETTINGS,
};
use windows::core::Interface;

fn gpu_facts() -> CaptureFacts {
    CaptureFacts {
        armed: Some(7),
        device: 7,
        disabled: false,
        video: true,
        rotated: false,
        scaled: false,
        duplication: false,
        overlay: false,
    }
}

#[test]
fn bildkette_task_win_gpu_capture_route_follows_the_ranks() {
    assert_eq!(capture_route(gpu_facts()), Ok(()));
    let duplication = CaptureFacts {
        duplication: true,
        ..gpu_facts()
    };
    // Desktop Duplication: the host blends the pointer unless the viewer
    // draws it (overlay); blending stays on the CPU path.
    assert_eq!(capture_route(duplication), Err(CpuReason::PointerBlended));
    let overlay = CaptureFacts {
        overlay: true,
        ..duplication
    };
    assert_eq!(capture_route(overlay), Ok(()));
    let cases = [
        (
            CaptureFacts {
                armed: None,
                ..gpu_facts()
            },
            CpuReason::NotArmed,
        ),
        (
            CaptureFacts {
                armed: Some(8),
                ..gpu_facts()
            },
            CpuReason::OtherDevice,
        ),
        (
            CaptureFacts {
                video: false,
                ..gpu_facts()
            },
            CpuReason::NoVideo,
        ),
        (
            CaptureFacts {
                rotated: true,
                ..gpu_facts()
            },
            CpuReason::Rotated,
        ),
        (
            CaptureFacts {
                scaled: true,
                ..gpu_facts()
            },
            CpuReason::Scaled,
        ),
        (
            CaptureFacts {
                disabled: true,
                ..overlay
            },
            CpuReason::Disabled,
        ),
        (
            CaptureFacts {
                disabled: true,
                armed: None,
                ..gpu_facts()
            },
            CpuReason::Disabled,
        ),
    ];
    for (facts, reason) in cases {
        assert_eq!(capture_route(facts), Err(reason), "{facts:?}");
    }
}

#[test]
fn bildkette_task_win_gpu_encoder_route_needs_device_awareness_and_adapter() {
    let facts = EncoderFacts {
        disabled: false,
        capture: true,
        video: true,
        aware: true,
        capture_adapter: Some(0x0000_0001_0000_00a0),
        encoder_adapter: Some(0x0000_0001_0000_00a0),
    };
    assert_eq!(encoder_route(facts), Ok(()));
    // An MFT without the adapter attribute is tried; a refusal of the
    // manager keeps system memory.
    let unknown = EncoderFacts {
        encoder_adapter: None,
        ..facts
    };
    assert_eq!(encoder_route(unknown), Ok(()));
    let cases = [
        (
            EncoderFacts {
                encoder_adapter: Some(0xb0),
                ..facts
            },
            SystemReason::OtherAdapter,
        ),
        (
            EncoderFacts {
                aware: false,
                ..facts
            },
            SystemReason::NotD3D11Aware,
        ),
        (
            EncoderFacts {
                capture: false,
                ..facts
            },
            SystemReason::NoCapture,
        ),
        (
            EncoderFacts {
                video: false,
                ..facts
            },
            SystemReason::NoVideo,
        ),
        (
            EncoderFacts {
                disabled: true,
                ..facts
            },
            SystemReason::Disabled,
        ),
    ];
    for (facts, reason) in cases {
        assert_eq!(encoder_route(facts), Err(reason), "{facts:?}");
    }
}

fn pattern(size: FrameSize, seed: u32) -> BgraImage {
    let mut state = seed.wrapping_mul(2_654_435_761).max(1);
    let mut data = vec![0u8; size.bgra_len()];
    for byte in data.iter_mut() {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        *byte = state as u8;
    }
    BgraImage::from_vec(size, data).expect("pattern")
}

#[test]
fn bildkette_task_win_gpu_cpu_conversion_is_bit_identical_to_the_reference() {
    let sizes = [(2, 2), (16, 8), (17, 9), (33, 31), (640, 360), (1, 3)];
    for (index, (width, height)) in sizes.into_iter().enumerate() {
        let size = FrameSize::new(width, height);
        let image = pattern(size, index as u32 + 1);
        let reference =
            bgra_to_i420_with_limits(&image.view(), YuvMatrix::Bt709, FrameLimits::LEGACY)
                .expect("reference");
        let chroma = size.chroma();
        for pitch in [(chroma.width * 2).max(width) as usize, width as usize + 64] {
            let pitch = pitch.max(chroma.width as usize * 2);
            let mut y = vec![0u8; pitch * height as usize];
            let mut uv = vec![0u8; pitch * chroma.height as usize];
            bgra_to_nv12(&image.view(), &mut y, &mut uv, pitch).expect("convert");
            for row in 0..height as usize {
                let start = row * width as usize;
                assert_eq!(
                    &y[row * pitch..row * pitch + width as usize],
                    &reference.y[start..start + width as usize],
                    "luma {size:?} row {row} pitch {pitch}"
                );
            }
            for row in 0..chroma.height as usize {
                for column in 0..chroma.width as usize {
                    let at = row * chroma.width as usize + column;
                    let pair = &uv[row * pitch + column * 2..row * pitch + column * 2 + 2];
                    assert_eq!(pair, [reference.u[at], reference.v[at]], "chroma {size:?}");
                }
            }
        }
    }
    // Planes too small for the picture are refused, not overrun.
    let image = pattern(FrameSize::new(8, 8), 9);
    let (mut y, mut uv) = (vec![0u8; 63], vec![0u8; 32]);
    assert!(bgra_to_nv12(&image.view(), &mut y, &mut uv, 8).is_err());
}

#[test]
fn bildkette_task_win_gpu_picture_of_a_retired_ring_is_never_read_as_pixels() {
    use crate::screen::core::capture_picture::{GpuPicture, GpuSurface, Picture};
    use std::sync::{Arc, Weak};
    // BK3b: the GPU picture names its ring slot; there is no placeholder in
    // system memory any more. A picture whose ring was retired is no longer
    // held: its readback is the skip error, never pixels.
    let size = FrameSize::new(1921, 1081);
    let grant = crate::screen::core::capture_test_support::grant(size, None);
    let surface = RingPicture {
        ring: Weak::new(),
        tag: 300,
        size: size.even(),
    };
    assert!(!surface.held());
    assert_eq!(surface.slot(), 300);
    let picture =
        GpuPicture::new(Arc::new(surface), size, FrameLimits::LEGACY, &grant).expect("GPU picture");
    assert_eq!(picture.size(), size);
    let error = picture.read_back().expect_err("no pixels without the ring");
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    let picture = Picture::Gpu(picture);
    assert!(picture.to_cpu().is_err(), "the only way to CPU pixels");
    // Scaling a GPU picture does no pixel work and keeps it on the GPU.
    let scaled = picture
        .scaled(
            FrameSize::new(1280, 720),
            FrameLimits::LEGACY,
            crate::screen::core::capture_admission::CaptureOwner::ScaledPixels,
        )
        .expect("scaled");
    assert!(scaled.on_gpu());
    assert_eq!(scaled.size(), FrameSize::new(1280, 720));
    let Picture::Gpu(gpu) = &scaled else {
        panic!("stays on the GPU");
    };
    assert_eq!(ring_picture(gpu).expect("ring slot").tag, 300);
}

#[test]
fn bildkette_task_win_gpu_ring_slot_choice() {
    assert_eq!(next_tag(0), 1);
    assert_eq!(next_tag(TAG_COUNT), 1);
    assert!(newer(5, 4) && !newer(4, 5) && !newer(4, 4));
    assert!(newer(1, TAG_COUNT), "wraps around");
    assert!(!newer(0, 3) && !newer(3, 0), "empty slots are never newer");
    assert_eq!(pick(7, [7, 8]), Some(0));
    assert_eq!(pick(8, [7, 8]), Some(1));
    // Overwritten: the newest later picture (the screen as it is now).
    assert_eq!(pick(6, [7, 8]), Some(1));
    assert_eq!(pick(6, [9, 8]), Some(0));
    assert_eq!(pick(6, [0, 7]), Some(1));
    // Only older pictures or none: the frame is skipped.
    assert_eq!(pick(9, [7, 8]), None);
    assert_eq!(pick(3, [0, 0]), None);
}

#[test]
fn bildkette_task_win_gpu_encoder_settings_mapping() {
    assert_eq!(SETTINGS.len(), 8);
    // B-29: the full-power state is asked for and listed when refused.
    assert_eq!(SETTINGS[7], "low_power_off");
    assert_eq!(one_frame_buffer(6_000_000, 60), 100_000);
    assert_eq!(one_frame_buffer(6_000_000, 0), 6_000_000);
    assert_eq!(ltr_control(2), 0x0001_0002);
    assert_eq!(gop_candidates(None), vec![u32::MAX, 65_535, 32_767]);
    assert_eq!(gop_candidates(Some(1_000))[0], 1_000);
    assert_eq!(gop_candidates(Some(0)), vec![u32::MAX, 65_535, 32_767]);
    assert_eq!(level_max_bitrate(51, false), Some(240_000_000));
    assert_eq!(level_max_bitrate(51, true), Some(300_000_000));
    assert_eq!(level_max_bitrate(62, true), Some(1_000_000_000));
    assert_eq!(level_max_bitrate(99, true), None);
    // No fixed 240 Mbit/s: the encoder and its level bound the rate.
    assert_eq!(bitrate_limit(None, None), u32::MAX);
    assert_eq!(
        bitrate_limit(Some(300_000_000), Some(100_000_000)),
        100_000_000
    );
    assert_eq!(bitrate_limit(Some(300_000_000), Some(0)), 300_000_000);
    assert_eq!(bounded_bitrate(500_000_000, u32::MAX), 500_000_000);
    assert_eq!(bounded_bitrate(500_000_000, 300_000_000), 300_000_000);
    assert_eq!(bounded_bitrate(10, u32::MAX), 150_000);
}

#[test]
fn bildkette_task_win_gpu_colour_space_matches_the_cpu_path() {
    // RGB full range in; BT.709 (bit 2) with nominal range 16-235 (bits 4-5
    // = 1) out, as `YuvMatrix::Bt709` of the CPU reference.
    assert_eq!(RGB_FULL_RANGE, 0);
    assert_eq!(YCBCR_BT709_LIMITED, 0b01_0100);
    assert_eq!(nv12_len(FrameSize::new(1920, 1080)), 3_110_400);
    assert_eq!(
        slot_size(FrameSize::new(1001, 701)),
        FrameSize::new(1000, 700)
    );
}

#[test]
fn bildkette_task_win_gpu_without_video_support_stays_on_cpu() {
    // The capture of this machine (a runner without GPU gets WARP): only a
    // device with video support may take the GPU route.
    let Ok(devices) = crate::screen::os::d3d_device::create() else {
        return;
    };
    let raw = devices.device.as_raw() as usize;
    let facts = CaptureFacts {
        armed: Some(raw),
        device: raw,
        video: devices.has_video(),
        ..gpu_facts()
    };
    let expected = if devices.has_video() {
        Ok(())
    } else {
        Err(CpuReason::NoVideo)
    };
    assert_eq!(capture_route(facts), expected);
    let encoder = EncoderFacts {
        disabled: false,
        capture: true,
        video: devices.has_video(),
        aware: true,
        capture_adapter: devices.adapter_luid(),
        encoder_adapter: devices.adapter_luid(),
    };
    assert_eq!(encoder_route(encoder).is_ok(), devices.has_video());
}

#[test]
#[ignore = "needs a GPU with a D3D11 video processor; Suite keine (kein Runner mit Grafikkarte)"]
fn bildkette_task_win_gpu_video_processor_matches_the_cpu_colours() {
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_SUBRESOURCE_DATA,
        D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    let devices = crate::screen::os::d3d_device::create().expect("D3D11 device");
    assert!(devices.has_video(), "device without video support");
    let size = FrameSize::new(64, 64);
    let colours = [
        [0, 0, 0, 255],
        [255, 255, 255, 255],
        [200, 140, 30, 255],
        [20, 60, 200, 255],
    ];
    let mut data = vec![0u8; size.bgra_len()];
    for (index, pixel) in data.chunks_exact_mut(4).enumerate() {
        let (x, y) = (index % 64, index / 64);
        pixel.copy_from_slice(&colours[(y / 32) * 2 + x / 32]);
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: 64,
        Height: 64,
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
        SysMemPitch: 64 * 4,
        SysMemSlicePitch: 0,
    };
    let mut source = None;
    // SAFETY: `data` outlives the call and holds 64 rows of 256 bytes.
    unsafe {
        devices
            .device
            .CreateTexture2D(&desc, Some(&initial), Some(&mut source))
    }
    .expect("source texture");
    let source = source.expect("source texture");
    let converter = VideoConverter::new(
        &devices.device,
        &devices.context,
        size,
        DXGI_FORMAT_B8G8R8A8_UNORM,
        size,
    )
    .expect("video processor");
    let target = nv12_texture(&devices.device, size, None).expect("NV12 target");
    let view = converter.output_view(&target).expect("output view");
    converter.convert(&source, &view).expect("convert");
    let staging =
        nv12_texture(&devices.device, size, Some(D3D11_CPU_ACCESS_READ)).expect("staging");
    // SAFETY: two live NV12 textures of the same size and device.
    unsafe { devices.context.CopyResource(&staging, &target) };
    let mut gpu = vec![0u8; nv12_len(size) as usize];
    with_mapped(
        &devices.context,
        &staging,
        size,
        D3D11_MAP_READ,
        |y, uv, pitch| {
            let (luma, chroma) = gpu.split_at_mut(64 * 64);
            for (row, out) in luma.chunks_exact_mut(64).enumerate() {
                out.copy_from_slice(&y[row * pitch..row * pitch + 64]);
            }
            for (row, out) in chroma.chunks_exact_mut(64).enumerate() {
                out.copy_from_slice(&uv[row * pitch..row * pitch + 64]);
            }
            Ok(())
        },
    )
    .expect("read back");
    let image = BgraImage::from_vec(size, data.clone()).expect("image");
    let mut cpu = vec![0u8; nv12_len(size) as usize];
    let (y, uv) = cpu.split_at_mut(64 * 64);
    bgra_to_nv12(&image.view(), y, uv, 64).expect("reference");
    // Centres of the uniform quadrants: same BT.709 limited values within
    // the rounding of the driver (siting differs only at edges).
    for (x, y) in [(16usize, 16usize), (48, 16), (16, 48), (48, 48)] {
        let luma = y * 64 + x;
        assert!(gpu[luma].abs_diff(cpu[luma]) <= 2, "luma at {x},{y}");
        let chroma = 64 * 64 + (y / 2) * 64 + (x / 2) * 2;
        assert!(gpu[chroma].abs_diff(cpu[chroma]) <= 3, "Cb at {x},{y}");
        assert!(
            gpu[chroma + 1].abs_diff(cpu[chroma + 1]) <= 3,
            "Cr at {x},{y}"
        );
    }
}
