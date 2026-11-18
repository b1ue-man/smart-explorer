//! BK9 (plan 2026-11-14 „Bildkette“): the display path from window backend
//! and decoder stage, the fallback, and the window configuration per mode.
use super::*;

const BACKENDS: [WindowBackend; 5] = [
    WindowBackend::Vulkan,
    WindowBackend::Dx12,
    WindowBackend::WgpuGl,
    WindowBackend::Glow,
    WindowBackend::Other,
];
const SURFACES: [DecodedSurface; 3] = [
    DecodedSurface::Memory,
    DecodedSurface::D3d11Texture,
    DecodedSurface::VaSurface,
];

#[test]
fn bildkette_task_present_path_from_backend_and_stage() {
    // No backend of the locked versions imports a decoded surface: every
    // picture that comes as planes is uploaded as planes, the rest as BGRA.
    for backend in BACKENDS {
        for surface in SURFACES {
            assert!(!imports(backend, surface), "{backend:?} {surface:?}");
            assert_eq!(
                select(backend, surface, true, false),
                PresentPath::PlaneUpload
            );
            assert_eq!(
                select(backend, surface, false, false),
                PresentPath::BgraUpload
            );
        }
    }
    assert_eq!(
        WindowBackend::of_wgpu(wgpu::Backend::Vulkan),
        WindowBackend::Vulkan
    );
    assert_eq!(
        WindowBackend::of_wgpu(wgpu::Backend::Dx12),
        WindowBackend::Dx12
    );
    assert_eq!(
        WindowBackend::of_wgpu(wgpu::Backend::Gl),
        WindowBackend::WgpuGl
    );
    assert_eq!(
        WindowBackend::of_wgpu(wgpu::Backend::Metal),
        WindowBackend::Other
    );
    assert_eq!(
        DecodedSurface::of_decoder("H264 Decoder MFT [Hardware (D3D11VA)]"),
        DecodedSurface::D3d11Texture
    );
    assert_eq!(
        DecodedSurface::of_decoder("vaapi-renderD128"),
        DecodedSurface::VaSurface
    );
    assert_eq!(
        DecodedSurface::of_decoder("openh264"),
        DecodedSurface::Memory
    );
    assert_eq!(DecodedSurface::of_decoder(""), DecodedSurface::Memory);
}

#[test]
fn bildkette_task_present_fallback_after_failed_import() {
    // A failed import keeps the session on the plane path (or BGRA where
    // the window does not take planes), whatever the backend would import.
    for backend in BACKENDS {
        for surface in SURFACES {
            assert_eq!(
                select(backend, surface, true, true),
                PresentPath::PlaneUpload
            );
            assert_eq!(
                select(backend, surface, false, true),
                PresentPath::BgraUpload
            );
        }
    }
}

#[test]
fn bildkette_task_present_stats_line_names_path() {
    assert_eq!(
        stats_line(
            PresentPath::PlaneUpload,
            WindowBackend::Vulkan,
            DecodedSurface::D3d11Texture
        ),
        "Anzeige Upload YUV-Ebenen (Vulkan, Hardwarebild zurückgelesen)"
    );
    assert_eq!(
        stats_line(
            PresentPath::BgraUpload,
            WindowBackend::Glow,
            DecodedSurface::Memory
        ),
        "Anzeige Upload BGRA (OpenGL (glow))"
    );
    assert_eq!(
        stats_line(
            PresentPath::Direct,
            WindowBackend::Dx12,
            DecodedSurface::D3d11Texture
        ),
        "Anzeige direkt (DX12)"
    );
}

#[test]
fn bildkette_task_present_configuration_per_mode() {
    let standard = wgpu_configuration(false);
    assert_eq!(standard.present_mode, wgpu::PresentMode::AutoVsync);
    assert_eq!(standard.desired_maximum_frame_latency, Some(1));
    let lowest = wgpu_configuration(true);
    assert_eq!(lowest.present_mode, wgpu::PresentMode::AutoNoVsync);
    assert_eq!(lowest.desired_maximum_frame_latency, Some(1));
    // Mailbox would end the program on a surface without it.
    for config in [&standard, &lowest] {
        assert_ne!(config.present_mode, wgpu::PresentMode::Mailbox);
        assert_eq!(
            config.supported_backends,
            egui_wgpu::WgpuConfiguration::default().supported_backends
        );
    }
    assert_eq!(present_mode(false), standard.present_mode);
    assert_eq!(present_mode(true), lowest.present_mode);
}

/// P8: the Direct3D 11 presenter takes D3D11VA textures as they are; FFmpeg's
/// stage names choose the surface, DXVA2 and software pictures are read
/// back or uploaded as before.
#[test]
fn bildkette_task_present_direct3d11_takes_d3d11va_textures() {
    let d3d11 = WindowBackend::Direct3d11;
    assert!(imports(d3d11, DecodedSurface::D3d11Texture));
    for surface in [
        DecodedSurface::Memory,
        DecodedSurface::VaSurface,
        DecodedSurface::Dxva2Surface,
    ] {
        assert!(!imports(d3d11, surface), "{surface:?}");
        assert_eq!(
            select(d3d11, surface, true, false),
            PresentPath::PlaneUpload
        );
    }
    assert_eq!(
        select(d3d11, DecodedSurface::D3d11Texture, true, false),
        PresentPath::Direct
    );
    // A failed presenter goes back to the planes.
    assert_eq!(
        select(d3d11, DecodedSurface::D3d11Texture, true, true),
        PresentPath::PlaneUpload
    );
    assert_eq!(
        DecodedSurface::of_decoder("FFmpeg h264 [D3D11VA]"),
        DecodedSurface::D3d11Texture
    );
    assert_eq!(
        DecodedSurface::of_decoder("FFmpeg hevc [DXVA2]"),
        DecodedSurface::Dxva2Surface
    );
    assert_eq!(
        DecodedSurface::of_decoder("FFmpeg av1 [Software]"),
        DecodedSurface::Memory
    );
    assert_eq!(
        stats_line(PresentPath::Direct, d3d11, DecodedSurface::D3d11Texture),
        "Anzeige direkt (Direct3D 11)"
    );
    assert_eq!(
        stats_line(
            PresentPath::PlaneUpload,
            WindowBackend::Vulkan,
            DecodedSurface::Dxva2Surface
        ),
        "Anzeige Upload YUV-Ebenen (Vulkan, Hardwarebild zurückgelesen)"
    );
    // No wgpu or glow backend imports any of them (locked versions).
    for backend in BACKENDS {
        assert!(!imports(backend, DecodedSurface::Dxva2Surface));
    }
}

/// P9: „Niedrigste Latenz“ applies live where the Direct3D 11 display owns
/// its swap chain (tearing only with the switch and DXGI's support); the
/// hint about the next start stays only where the wgpu window shows the
/// picture.
#[test]
fn p9_present_task_lowest_latency_applies_live_on_direct3d11() {
    assert!(direct_allows_tearing(true, true));
    assert!(!direct_allows_tearing(true, false));
    assert!(!direct_allows_tearing(false, true));
    assert!(!direct_allows_tearing(false, false));
    assert_eq!(latency_note(false, false), None);
    assert_eq!(latency_note(false, true), None);
    assert_eq!(latency_note(true, true), None);
    assert_eq!(
        latency_note(true, false),
        Some("Wirkt beim nächsten Start des Betrachters.")
    );
    // The switch's value is what the presenter reads next.
    set_low_latency(true);
    assert!(low_latency());
    set_low_latency(false);
    assert!(!low_latency());
}
