//! Stable Android pane accounting. Public handles never expose crate-private pixels or slices.
use std::{collections::BTreeMap, io, sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}}};
use crate::{login::{FrameToken, ViewerFrame}, screen::core::{
    capture_admission::CaptureSlice, frame_memory::CapacityCeiling,
    image_admission::{self, ImageAdmission, ImageOwnerLimits, ImageRole, LocalImagePolicy},
}};
const MAX_IMAGE: u64 = crate::login::core::model::MAX_PIXELS as u64;
const MAX_TARGET: u64 = 64 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(1);
static PANES: Mutex<BTreeMap<u64, Arc<Pane>>> = Mutex::new(BTreeMap::new());
struct Binding {session: u64, generation: u64, closed: bool}
pub(super) struct Pane {images: Arc<ImageAdmission>, binding: Mutex<Binding>}
fn invalid(text: &str) -> io::Error {io::Error::new(io::ErrorKind::InvalidInput, text)}
fn next() -> io::Result<u64> {
    NEXT.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |id| id.checked_add(1).filter(|v| *v <= i64::MAX as u64))
        .map_err(|_| invalid("login image identifiers exhausted"))
}
fn pane(id: u64) -> io::Result<Arc<Pane>> {
    PANES.lock().map_err(|_| io::Error::other("login image registry unavailable"))?
        .get(&id).cloned().ok_or_else(|| invalid("unknown login image owner"))
}
pub fn login_image_owner_create(heap_offer: u64) -> u64 {
    create(heap_offer).unwrap_or(0)
}
fn create(heap_offer: u64) -> io::Result<u64> {
    if heap_offer == 0 {return Err(invalid("Android heap offer is empty"));}
    let ceiling = CapacityCeiling::bounded(heap_offer);
    crate::screen::os::image_memory::observe_capacity_ceiling(ceiling)?;
    let metadata = std::mem::size_of::<LoginBitmapTicket>() as u64;
    let limits = ImageOwnerLimits::new(4 * MAX_IMAGE + 2 * MAX_TARGET + 2 * metadata)?
        .with_role(ImageRole::ViewerInput, MAX_IMAGE, 2)?
        .with_role(ImageRole::AndroidBitmap, MAX_IMAGE, 2)?
        .with_role(ImageRole::RenderSurface, MAX_TARGET, 2)?
        .with_role(ImageRole::ImageMetadata, metadata, 2)?;
    let images = image_admission::admit_images(LocalImagePolicy {facts: None, ceiling}, limits)?;
    let mut panes = PANES.lock().map_err(|_| io::Error::other("login image registry unavailable"))?;
    panes.retain(|_, pane| {
        let closed = pane.binding.lock().is_ok_and(|state| state.closed);
        !(closed && pane.images.live_bytes().is_ok_and(|bytes| bytes == 0))
    });
    if panes.len() >= 64 {return Err(io::Error::new(io::ErrorKind::WouldBlock, "login image pane admission full"));}
    let id = next()?;
    panes.insert(id, Arc::new(Pane {images, binding: Mutex::new(Binding {session: 0, generation: 0, closed: false})}));
    Ok(id)
}
pub(super) fn bind(owner: u64, session: u64, heap_offer: u64) -> io::Result<Arc<ImageAdmission>> {
    if heap_offer == 0 {return Err(invalid("Android heap offer is required before login Open"));}
    crate::screen::os::image_memory::observe_capacity_ceiling(CapacityCeiling::bounded(heap_offer))?;
    let pane = pane(owner)?;
    let mut state = pane.binding.lock().map_err(|_| io::Error::other("login pane binding unavailable"))?;
    if state.closed {return Err(invalid("login image pane is closed"));}
    state.generation = state.generation.checked_add(1).ok_or_else(|| invalid("login image generation exhausted"))?;
    state.session = session;
    Ok(pane.images.clone())
}
pub(super) fn unbind(session: u64) {
    if let Ok(panes) = PANES.lock() {for pane in panes.values() {
        if let Ok(mut state) = pane.binding.lock() {if state.session == session {state.session = 0;}}
    }}
}
pub fn login_image_owner_close(owner: u64) {
    let session = pane(owner).ok().and_then(|pane| {
        let mut state = pane.binding.lock().ok()?;
        state.closed = true;
        let session = state.session; state.session = 0;
        Some(session)
    });
    if let Some(session) = session.filter(|id| *id != 0) {super::login_view::login_close(session);}
    // Closed panes with live retirement stay in the bounded registry. A later
    // create prunes them only after every actual image/target claim has ended.
}
/// The raw ViewerFrame and both claims precede Bitmap.createBitmap. No bytes escape.
pub struct LoginBitmapTicket {
    frame: ViewerFrame,
    lease: LoginBitmapLease,
}
pub struct LoginBitmapLease {
    pane: Arc<Pane>,
    pixels: CaptureSlice,
    _metadata: CaptureSlice,
    owner: u64,
    picture: u64,
    generation: u64,
    session: u64,
}
impl LoginBitmapLease {
    pub fn owner(&self) -> u64 {self.owner}
    pub fn picture(&self) -> u64 {self.picture}
    pub fn generation(&self) -> u64 {self.generation}
    pub fn is_current(&self) -> bool {
        self.pane.binding.lock().is_ok_and(|state| !state.closed && state.session == self.session && state.generation == self.generation)
    }
    pub fn bytes(&self) -> u64 {self.pixels.bytes()}
}
impl LoginBitmapTicket {
    pub fn width(&self) -> u32 {self.frame.token.width}
    pub fn height(&self) -> u32 {self.frame.token.height}
    pub fn descriptor(&self) -> String {
        serde_json::json!({"owner":self.lease.owner,"picture":self.lease.picture,"generation":self.lease.generation,"token":self.frame.token}).to_string()
    }
    pub fn is_current(&self) -> bool {self.lease.is_current()}
    /// A synchronous NDK lock lends only its actual strided destination.
    pub fn fill_rgba(&self, target: &mut [u8], stride: usize) -> io::Result<()> {
        let row = self.width() as usize * 4;
        let needed = stride.checked_mul(self.height() as usize).ok_or_else(|| invalid("bitmap stride overflow"))?;
        if stride < row || target.len() < needed {return Err(invalid("bitmap destination is too short"));}
        let source = self.frame.image.view();
        for y in 0..self.height() {
            let dst = &mut target[y as usize * stride..y as usize * stride + row];
            for (from, to) in source.row(y).chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                to.copy_from_slice(&[from[2], from[1], from[0], 255]);
            }
        }
        Ok(())
    }
    pub fn finish(self) -> LoginBitmapLease {self.lease}
}
pub fn login_bitmap_take(session: u64, owner: u64) -> Result<Option<LoginBitmapTicket>, String> {
    take(session, owner).map_err(|error| error.to_string())
}
fn take(session: u64, owner: u64) -> io::Result<Option<LoginBitmapTicket>> {
    let pane = pane(owner)?;
    let state = pane.binding.lock().map_err(|_| io::Error::other("login pane binding unavailable"))?;
    if state.closed || state.session != session {return Err(invalid("login image owner belongs to a different session"));}
    // Do not consume a raw frame until the two bitmap slots can accept it.
    if pane.images.live_count(ImageRole::AndroidBitmap)? >= 2 {return Ok(None);}
    let generation = state.generation;
    drop(state); // never hold a pane lock while acquiring the session registry
    let Some(frame) = super::login_view::take_image_frame(session) else {return Ok(None);};
    let state = pane.binding.lock().map_err(|_| io::Error::other("login pane binding unavailable"))?;
    if state.closed || state.session != session || state.generation != generation {return Ok(None);}
    let result = admit_frame(pane.clone(), owner, session, generation, frame);
    drop(state);
    result
}
fn admit_frame(pane: Arc<Pane>, owner: u64, session: u64, generation: u64, frame: ViewerFrame) -> io::Result<Option<LoginBitmapTicket>> {
    let token: &FrameToken = &frame.token;
    let size = frame.image.size();
    let bytes = u64::from(token.width).checked_mul(u64::from(token.height)).and_then(|n| n.checked_mul(4))
        .filter(|n| *n > 0 && *n <= MAX_IMAGE).ok_or_else(|| invalid("login bitmap geometry exceeds limits"))?;
    if token.frame_id == 0 || token.width == 0 || token.height == 0 || token.width > 4096 || token.height > 4096
        || size.width != token.width || size.height != token.height || token.digest.len() != 64
        || !token.digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || frame.image.image_role() != Some(ImageRole::ViewerInput)
        || !frame.image.image_admission().is_some_and(|images| Arc::ptr_eq(&images, &pane.images)) {
        return Err(invalid("login bitmap has a foreign token, geometry or raw claim"));
    }
    let pixels = match pane.images.try_claim(ImageRole::AndroidBitmap, bytes) {
        Ok(slice) => slice, Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None), Err(error) => return Err(error),
    };
    let metadata = pane.images.try_claim(ImageRole::ImageMetadata, std::mem::size_of::<LoginBitmapTicket>() as u64)?;
    let lease = LoginBitmapLease {pane: pane.clone(), pixels, _metadata: metadata, owner, picture: next()?, generation, session};
    Ok(Some(LoginBitmapTicket {frame, lease}))
}
/// The target claim stays alive through HardwareRenderer.destroy and Surface.release.
pub struct LoginRenderTarget { _pane: Arc<Pane>, _pixels: CaptureSlice }
pub fn login_render_target_claim(owner: u64, width: u32, height: u32) -> Option<LoginRenderTarget> {
    let pane = pane(owner).ok()?;
    let state = pane.binding.lock().ok()?;
    if state.closed {return None;}
    let bytes = u64::from(width).checked_mul(u64::from(height))?.checked_mul(4)?;
    if width == 0 || height == 0 || bytes > MAX_TARGET {return None;}
    let pixels = pane.images.try_claim(ImageRole::RenderSurface, bytes).ok()?;
    drop(state);
    Some(LoginRenderTarget {_pane: pane, _pixels: pixels})
}

#[cfg(test)]
#[path = "batches_login_image_task_tests.rs"]
mod batches_login_image_task_tests;

/// Read-only fixture diagnostics retain no bitmap or slice aliases.
#[cfg(all(target_os = "android", feature = "android-instrumentation-fixtures"))]
pub(super) fn fixture_state(owner: u64) -> io::Result<serde_json::Value> {
    fixture_pane_state(owner, &pane(owner)?)
}
#[cfg(all(target_os = "android", feature = "android-instrumentation-fixtures"))]
pub(super) fn fixture_pane(owner: u64) -> io::Result<Arc<Pane>> {pane(owner)}
#[cfg(all(target_os = "android", feature = "android-instrumentation-fixtures"))]
pub(super) fn fixture_pane_state(owner: u64, pane: &Arc<Pane>) -> io::Result<serde_json::Value> {
    let state = pane.binding.lock().map_err(|_| io::Error::other("login pane binding unavailable"))?;
    Ok(serde_json::json!({"owner":owner,"session":state.session,"generation":state.generation,"closed":state.closed,
        "live_bytes":pane.images.live_bytes()?,"raw_count":pane.images.live_count(ImageRole::ViewerInput)?,
        "bitmap_count":pane.images.live_count(ImageRole::AndroidBitmap)?,"target_count":pane.images.live_count(ImageRole::RenderSurface)?,
        "metadata_count":pane.images.live_count(ImageRole::ImageMetadata)?}))
}
