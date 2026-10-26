//! The original painter supplies checked upload/draw; completion never implies image correctness.
use super::screen_render_upload::{Drain, PreparedImage};
use eframe::{
    egui,
    egui_glow::{
        self,
        glow::{self, HasContext},
        GlErrorScope, GlOperationError,
    },
};
use std::{
    ffi::{c_void, CStr},
    num::NonZeroU32,
    sync::Arc,
};
// Only the same native context's loader supplies this core GL entrypoint.
type GenTextures = unsafe extern "system" fn(i32, *mut u32);
pub(super) fn native_texture(create: GenTextures) -> Result<glow::Texture, GlOperationError> {
    let mut name = 0;
    unsafe {
        create(1, &mut name);
    }
    NonZeroU32::new(name)
        .map(glow::NativeTexture)
        .ok_or_else(GlOperationError::texture_creation_failed)
}
struct SyncPoint(usize);
#[derive(Clone, Copy)]
pub(super) struct GlowImage {
    pub id: egui::TextureId,
    pub size: [usize; 2],
}
pub(super) struct GlowUpload {
    pub image: GlowImage,
    pub result: Result<(), GlOperationError>,
    pub drain: Drain,
}
pub(crate) struct GlowBackend {
    painter: egui_glow::Painter,
    create: GenTextures,
    next: u64,
    fences: Vec<(SyncPoint, Drain)>,
    quarantined: bool,
    error: Option<GlOperationError>,
}
impl GlowBackend {
    pub(crate) fn new(
        gl: Arc<glow::Context>,
        get_proc: Option<&dyn Fn(&CStr) -> *const c_void>,
        shader: Option<egui_glow::ShaderVersion>,
        dithering: bool,
    ) -> Result<Self, String> {
        let name = CStr::from_bytes_with_nul(b"glGenTextures\0").map_err(|e| e.to_string())?;
        let address = get_proc.ok_or_else(|| "GL context has no function loader".to_owned())?(name);
        if address.is_null() {
            return Err("GL context has no glGenTextures".into());
        }
        let create = unsafe { std::mem::transmute::<*const c_void, GenTextures>(address) };
        egui_glow::Painter::new(gl, "", shader, dithering)
            .map(|painter| Self {
                painter,
                create,
                next: 1,
                fences: Vec::new(),
                quarantined: false,
                error: None,
            })
            .map_err(|e| e.to_string())
    }
    pub(crate) fn available(&self) -> bool {
        !self.quarantined
    }
    pub(crate) fn context_matches(&self, gl: &glow::Context) -> bool {
        std::ptr::eq(gl, self.painter.gl().as_ref())
    }
    pub(crate) fn texture_limit(&self) -> u32 {
        self.painter.max_texture_side().min(u32::MAX as usize) as u32
    }
    fn observe(&mut self, error: GlOperationError) {
        eprintln!("Bildschirmrenderer: {error}");
        self.error = Some(error);
        self.quarantined |= error.state_unreliable();
    }
    fn quarantine(&mut self, error: GlOperationError) {
        self.observe(error);
        self.quarantined = true;
    }
    pub(crate) fn upload(
        &mut self,
        existing: Option<GlowImage>,
        image: &PreparedImage,
        failed: impl Fn(),
    ) -> GlowUpload {
        let id = existing.map(|image| image.id).unwrap_or_else(|| {
            let id = egui::TextureId::Managed(self.next);
            self.next += 1;
            id
        });
        let mut result = if self.available() {
            let create = self.create;
            image
                .image
                .upload_glow(&mut self.painter, id, || native_texture(create))
        } else {
            Err(self
                .error
                .unwrap_or_else(GlOperationError::texture_creation_failed))
        };
        // The completion fallback can call glFinish. Revoke the old displayed
        // proof as soon as upload fails, before waiting for that completion.
        if let Err(error) = result {
            failed();
            self.observe(error);
        }
        let drain = self.checkpoint();
        if !self.available() {
            failed();
            result = Err(self
                .error
                .unwrap_or_else(GlOperationError::texture_creation_failed));
        }
        GlowUpload {
            image: GlowImage {
                id,
                size: image.image.size,
            },
            result,
            drain,
        }
    }
    pub(crate) fn draw(
        &mut self,
        id: egui::TextureId,
        rect: egui::Rect,
        clip: egui::Rect,
        info: egui::PaintCallbackInfo,
    ) -> Result<(), GlOperationError> {
        if !self.available() {
            return Err(self
                .error
                .unwrap_or_else(GlOperationError::texture_creation_failed));
        }
        let mut mesh = egui::Mesh::with_texture(id);
        mesh.add_rect_with_uv(
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        let result = self.painter.try_paint_primitives(
            info.screen_size_px,
            info.pixels_per_point,
            &[egui::ClippedPrimitive {
                clip_rect: clip.intersect(info.clip_rect),
                primitive: egui::epaint::Primitive::Mesh(mesh),
            }],
        );
        if let Err(error) = result {
            self.observe(error);
        }
        result
    }
    pub(crate) fn retire(&mut self, id: egui::TextureId) -> Drain {
        if !self.available() {
            return Drain::default();
        }
        let gl = self.painter.gl().clone();
        let scope = match GlErrorScope::begin(&gl) {
            Ok(scope) => scope,
            Err(error) => {
                self.quarantine(error);
                return Drain::default();
            }
        };
        self.painter.free_texture(id);
        if let Err(error) = scope.finish(Ok(())) {
            self.quarantine(error);
            return Drain::default();
        }
        self.checkpoint()
    }
    pub(crate) fn checkpoint(&mut self) -> Drain {
        let drain = Drain::default();
        if !self.available() {
            return drain;
        }
        let gl = self.painter.gl().clone();
        let version = gl.version();
        let scope = match GlErrorScope::begin(&gl) {
            Ok(scope) => scope,
            Err(error) => {
                self.quarantine(error);
                return drain;
            }
        };
        let sync = if version.is_embedded {
            version.major >= 3
        } else {
            version.major > 3
                || (version.major == 3 && version.minor >= 2)
                || gl.supported_extensions().contains("GL_ARB_sync")
        };
        if sync {
            let fence = unsafe { gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0) }
                .ok()
                .filter(|fence| !fence.0.is_null());
            if fence.is_some() {
                unsafe {
                    gl.flush();
                }
            }
            let result = scope.finish(if fence.is_some() {
                Ok(())
            } else {
                Err(GlOperationError::texture_creation_failed())
            });
            if let Some(fence) = fence {
                self.fences
                    .push((SyncPoint(fence.0 as usize), drain.clone()));
            }
            if let Err(error) = result {
                self.quarantine(error);
            }
        } else {
            unsafe {
                gl.finish();
            }
            match scope.finish(Ok(())) {
                Ok(()) => drain.confirm(),
                Err(error) => self.quarantine(error),
            }
        }
        drain
    }
    pub(crate) fn service(&mut self) {
        if !self.available() {
            return;
        }
        let gl = self.painter.gl().clone();
        let mut index = 0;
        while index < self.fences.len() {
            let scope = match GlErrorScope::begin(&gl) {
                Ok(scope) => scope,
                Err(error) => {
                    self.quarantine(error);
                    return;
                }
            };
            let fence = glow::NativeFence(self.fences[index].0 .0 as _);
            let result = unsafe { gl.client_wait_sync(fence, 0, 0) };
            let ready = result == glow::ALREADY_SIGNALED || result == glow::CONDITION_SATISFIED;
            if ready {
                unsafe {
                    gl.delete_sync(fence);
                }
            }
            let result = scope.finish(if result == glow::WAIT_FAILED {
                Err(GlOperationError::from_gl_code(glow::INVALID_OPERATION))
            } else {
                Ok(ready)
            });
            match result {
                Ok(true) => {
                    let (_, drain) = self.fences.swap_remove(index);
                    drain.confirm();
                }
                Ok(false) => index += 1,
                Err(error) => {
                    self.quarantine(error);
                    return;
                }
            }
        }
    }
    pub(crate) fn shutdown(&mut self) {
        if !self.available() {
            return;
        }
        let gl = self.painter.gl().clone();
        let scope = match GlErrorScope::begin(&gl) {
            Ok(scope) => scope,
            Err(error) => {
                self.quarantine(error);
                return;
            }
        };
        self.painter.destroy();
        for (fence, _) in &self.fences {
            unsafe {
                gl.delete_sync(glow::NativeFence(fence.0 as _));
            }
        }
        unsafe {
            gl.finish();
        }
        match scope.finish(Ok(())) {
            Ok(()) => {
                for (_, drain) in self.fences.drain(..) {
                    drain.confirm();
                }
            }
            Err(error) => self.quarantine(error),
        }
    }
}
