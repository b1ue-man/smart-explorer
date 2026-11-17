//! Direct3D 11 presenter of the Windows viewer (plan 2026-11-17 P8; core
//! side `core/screen_direct.rs`). It draws FFmpeg's D3D11VA textures with
//! the video processor (NV12 → BGRA, scaling, letterbox) into a flip-model
//! swap chain of a child window (`screen_d3d11_window.rs`), on the device
//! the decoder uses (`screen::os::viewer_d3d11`), under the device's lock.
//!
//! Swap chain as Moonlight-qt (`app/streaming/video/ffmpeg-renderers/
//! d3d11va.cpp:551-647` at 28629dc1): `DXGI_SWAP_EFFECT_FLIP_DISCARD`, five
//! buffers (three queued, one back buffer, one DWM holds for DirectFlip;
//! three starve AMD drivers there), `DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING` and
//! `Present(0, DXGI_PRESENT_ALLOW_TEARING)` where `CheckFeatureSupport`
//! reports tearing, else `Present(0, 0)` and DWM keeps the newest picture.
//! No waitable object and no `SetMaximumFrameLatency(1)`: with sync interval
//! 0 that makes `Present` block on DWM like sync interval 1 (Moonlight's
//! finding, same lines), which is the delay this path removes. Window
//! changes by DXGI are off (`DXGI_MWA_NO_WINDOW_CHANGES`), so presenting
//! from the decode thread never needs the window's thread. Swap chain
//! creation and `ResizeBuffers` happen on the window's thread
//! (`Surface::update`), under the same mutex as presenting.
use std::ffi::c_void;
use std::io;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use windows::core::Interface;
use windows::Win32::Foundation::{BOOL, HWND, RECT};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator,
    ID3D11VideoProcessorInputView, ID3D11VideoProcessorOutputView, D3D11_TEX2D_VPIV,
    D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_VIDEO_COLOR, D3D11_VIDEO_COLOR_0,
    D3D11_VIDEO_COLOR_RGBA, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
    D3D11_VIDEO_PROCESSOR_COLOR_SPACE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_UNSPECIFIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_RATIONAL,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIFactory2, IDXGIFactory5, IDXGIOutput, IDXGISwapChain1,
    DXGI_FEATURE_PRESENT_ALLOW_TEARING, DXGI_MWA_NO_WINDOW_CHANGES, DXGI_PRESENT,
    DXGI_PRESENT_ALLOW_TEARING, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

use super::layout::{blit_rects, colour_space_bits, PixelRect};
use super::DirectPresenter;
use crate::screen::core::gpu_picture::GpuPicture;
use crate::screen::core::pixel_convert::{FrameSize, YuvMatrix};
use crate::screen::os::ffmpeg_frame::D3d11Frame;
use crate::screen::os::viewer_d3d11::{self, ViewerDevice};

#[path = "screen_d3d11_window.rs"]
mod window;
pub use window::Surface;

/// Swap chain buffers (module head).
const BUFFERS: u32 = 5;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Processor {
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    /// Input texture size and output size it was made for.
    key: (u32, u32, u32, u32),
}

#[derive(Default)]
struct State {
    swap_chain: Option<IDXGISwapChain1>,
    /// Back buffer size.
    size: (u32, u32),
    /// The picture rectangle relative to the child.
    dest: PixelRect,
    processor: Option<Processor>,
    /// The picture drawn last (kept for redraws on resize).
    shown: Option<GpuPicture>,
    /// A picture not drawn yet, with the session it belongs to.
    held: Option<(GpuPicture, usize)>,
}

pub struct Presenter {
    device: Arc<ViewerDevice>,
    tearing: bool,
    state: Mutex<State>,
    active: AtomicBool,
    failed: AtomicBool,
}

// SAFETY: the D3D interfaces belong to the free-threaded viewer device and
// are used under `state`'s mutex and the device lock only.
unsafe impl Send for Presenter {}
// SAFETY: as above.
unsafe impl Sync for Presenter {}

fn factory(device: &ViewerDevice) -> io::Result<IDXGIFactory2> {
    // SAFETY: queries on the live device.
    unsafe {
        let dxgi: IDXGIDevice = device.device.cast()?;
        Ok(dxgi.GetAdapter()?.GetParent::<IDXGIFactory2>()?)
    }
}

fn tearing_supported(factory: &IDXGIFactory2) -> bool {
    let Ok(factory5) = factory.cast::<IDXGIFactory5>() else {
        return false;
    };
    let mut allowed = BOOL(0);
    // SAFETY: the out value and its size are passed together.
    let checked = unsafe {
        factory5.CheckFeatureSupport(
            DXGI_FEATURE_PRESENT_ALLOW_TEARING,
            &mut allowed as *mut BOOL as *mut c_void,
            std::mem::size_of::<BOOL>() as u32,
        )
    };
    checked.is_ok() && allowed.as_bool()
}

fn rect(rect: PixelRect) -> RECT {
    RECT {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

impl Presenter {
    pub fn new() -> io::Result<Arc<Self>> {
        let device = viewer_d3d11::shared()?;
        let tearing = tearing_supported(&factory(&device)?);
        Ok(Arc::new(Self {
            device,
            tearing,
            state: Mutex::new(State::default()),
            active: AtomicBool::new(false),
            failed: AtomicBool::new(false),
        }))
    }

    fn flags(&self) -> u32 {
        if self.tearing {
            DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 as u32
        } else {
            0
        }
    }

    /// On the window's thread: the swap chain of `hwnd` at `width` ×
    /// `height` with the picture at `dest`; redraws the shown picture when
    /// anything changed.
    pub(super) fn configure(
        &self,
        hwnd: HWND,
        width: u32,
        height: u32,
        dest: PixelRect,
    ) -> io::Result<()> {
        let mut state = lock(&self.state);
        let size = (width.max(1), height.max(1));
        let changed = state.size != size || state.dest != dest;
        if state.swap_chain.is_none() {
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: size.0,
                Height: size.1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: BUFFERS,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
                Flags: self.flags(),
            };
            let factory = factory(&self.device)?;
            // SAFETY: a live child window of this thread and a checked
            // description; DXGI keeps its own reference to the device.
            let swap_chain = unsafe {
                let swap_chain = factory.CreateSwapChainForHwnd(
                    &self.device.device,
                    hwnd,
                    &desc,
                    None,
                    None::<&IDXGIOutput>,
                )?;
                factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_WINDOW_CHANGES)?;
                swap_chain
            };
            state.swap_chain = Some(swap_chain);
        } else if state.size != size {
            let _guard = self.device.lock();
            if let Some(swap_chain) = &state.swap_chain {
                // SAFETY: no back buffer reference is held between frames.
                unsafe {
                    swap_chain.ResizeBuffers(
                        0,
                        size.0,
                        size.1,
                        DXGI_FORMAT_UNKNOWN,
                        DXGI_SWAP_CHAIN_FLAG(self.flags() as i32),
                    )?
                };
            }
        }
        state.size = size;
        state.dest = dest;
        if changed {
            if let Some(picture) = state.shown.take() {
                let drawn = self.draw(&mut state, &picture);
                state.shown = Some(picture);
                drawn?;
            }
        }
        Ok(())
    }

    /// Releases the swap chain (the window goes away).
    pub(super) fn close(&self) {
        self.failed.store(true, Ordering::Release);
        self.active.store(false, Ordering::Release);
        let mut state = lock(&self.state);
        state.processor = None;
        state.swap_chain = None;
        state.shown = None;
        state.held = None;
    }

    fn processor(&self, state: &mut State, input: (u32, u32)) -> io::Result<()> {
        let key = (input.0, input.1, state.size.0, state.size.1);
        if state.processor.as_ref().is_some_and(|p| p.key == key) {
            return Ok(());
        }
        let rate = DXGI_RATIONAL {
            Numerator: 60,
            Denominator: 1,
        };
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: rate,
            InputWidth: input.0,
            InputHeight: input.1,
            OutputFrameRate: rate,
            OutputWidth: state.size.0,
            OutputHeight: state.size.1,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
        };
        // SAFETY: a checked description on the live video device.
        let (enumerator, processor) = unsafe {
            let enumerator = self
                .device
                .video_device
                .CreateVideoProcessorEnumerator(&desc)?;
            let processor = self
                .device
                .video_device
                .CreateVideoProcessor(&enumerator, 0)?;
            (enumerator, processor)
        };
        state.processor = Some(Processor {
            enumerator,
            processor,
            key,
        });
        Ok(())
    }

    /// Draws `picture` into the back buffer and presents it.
    fn draw(&self, state: &mut State, picture: &GpuPicture) -> io::Result<()> {
        let frame = picture
            .surface
            .downcast_ref::<D3d11Frame>()
            .ok_or_else(|| io::Error::other("no D3D11 picture"))?;
        if !Arc::ptr_eq(frame.device(), &self.device) {
            return Err(io::Error::other("picture of another Direct3D 11 device"));
        }
        let Some(swap_chain) = state.swap_chain.clone() else {
            return Ok(());
        };
        let mut texture_desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: a live texture and an out structure.
        unsafe { frame.texture().GetDesc(&mut texture_desc) };
        let _guard = self.device.lock();
        self.processor(state, (texture_desc.Width, texture_desc.Height))?;
        let Some(processor) = state.processor.as_ref() else {
            return Ok(());
        };
        let (width, height) = (state.size.0 as i32, state.size.1 as i32);
        let rects = blit_rects(width, height, state.dest, frame.size());
        let video = &self.device.video_context;
        // SAFETY: views of live resources of the viewer device, created and
        // used under its lock; the stream's view reference is dropped after
        // the blit.
        unsafe {
            let back_buffer: ID3D11Texture2D = swap_chain.GetBuffer(0)?;
            let output_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let mut output: Option<ID3D11VideoProcessorOutputView> = None;
            self.device.video_device.CreateVideoProcessorOutputView(
                &back_buffer,
                &processor.enumerator,
                &output_desc,
                Some(&mut output as *mut Option<ID3D11VideoProcessorOutputView>),
            )?;
            let output = output.ok_or_else(|| io::Error::other("no output view"))?;
            let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPIV {
                        MipSlice: 0,
                        ArraySlice: frame.index(),
                    },
                },
            };
            let mut input: Option<ID3D11VideoProcessorInputView> = None;
            self.device.video_device.CreateVideoProcessorInputView(
                frame.texture(),
                &processor.enumerator,
                &input_desc,
                Some(&mut input as *mut Option<ID3D11VideoProcessorInputView>),
            )?;
            let vp = &processor.processor;
            let black = D3D11_VIDEO_COLOR {
                Anonymous: D3D11_VIDEO_COLOR_0 {
                    RGBA: D3D11_VIDEO_COLOR_RGBA {
                        R: 0.0,
                        G: 0.0,
                        B: 0.0,
                        A: 1.0,
                    },
                },
            };
            let bt709 = frame.matrix() == YuvMatrix::Bt709;
            let input_space = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
                _bitfield: colour_space_bits(Some((bt709, frame.full_range()))),
            };
            let output_space = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
                _bitfield: colour_space_bits(None),
            };
            video.VideoProcessorSetStreamColorSpace(vp, 0, &input_space);
            video.VideoProcessorSetOutputColorSpace(vp, &output_space);
            video.VideoProcessorSetStreamFrameFormat(vp, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
            video.VideoProcessorSetStreamAutoProcessingMode(vp, 0, false);
            video.VideoProcessorSetOutputBackgroundColor(vp, false, &black);
            let target = rect(PixelRect::new(0, 0, width, height));
            video.VideoProcessorSetOutputTargetRect(vp, true, Some(&target as *const RECT));
            let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: rects.is_some().into(),
                pInputSurface: ManuallyDrop::new(input),
                ..Default::default()
            };
            if let Some((source, dest)) = rects {
                let (source, dest) = (rect(source), rect(dest));
                video.VideoProcessorSetStreamSourceRect(vp, 0, true, Some(&source as *const RECT));
                video.VideoProcessorSetStreamDestRect(vp, 0, true, Some(&dest as *const RECT));
            }
            let blit = video.VideoProcessorBlt(vp, &output, 0, std::slice::from_ref(&stream));
            ManuallyDrop::drop(&mut stream.pInputSurface);
            blit?;
            let flags = if self.tearing {
                DXGI_PRESENT_ALLOW_TEARING
            } else {
                DXGI_PRESENT(0)
            };
            swap_chain.Present(0, flags).ok()?;
        }
        Ok(())
    }

    fn fail(&self, error: io::Error) -> io::Error {
        self.failed.store(true, Ordering::Release);
        self.active.store(false, Ordering::Release);
        error
    }
}

impl DirectPresenter for Presenter {
    fn present(&self, picture: GpuPicture, owner: usize) -> io::Result<bool> {
        let mut state = lock(&self.state);
        if state.swap_chain.is_none() {
            state.held = Some((picture, owner));
            return Ok(false);
        }
        self.draw(&mut state, &picture)
            .map_err(|error| self.fail(error))?;
        state.held = None;
        state.shown = Some(picture);
        self.active.store(true, Ordering::Release);
        Ok(true)
    }

    fn hold(&self, picture: GpuPicture, owner: usize) {
        lock(&self.state).held = Some((picture, owner));
    }

    fn show_held(&self, owner: usize) -> io::Result<bool> {
        let mut state = lock(&self.state);
        if state.swap_chain.is_none() || state.held.as_ref().is_none_or(|(_, of)| *of != owner) {
            return Ok(false);
        }
        let Some((picture, _)) = state.held.take() else {
            return Ok(false);
        };
        self.draw(&mut state, &picture)
            .map_err(|error| self.fail(error))?;
        state.shown = Some(picture);
        self.active.store(true, Ordering::Release);
        Ok(true)
    }

    fn picture_size(&self) -> Option<FrameSize> {
        let state = lock(&self.state);
        let held = state.held.as_ref().map(|(picture, _)| picture);
        state.shown.as_ref().or(held).map(|picture| picture.size)
    }

    fn active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    fn set_active(&self, on: bool) {
        self.active.store(on, Ordering::Release);
    }

    fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
}
