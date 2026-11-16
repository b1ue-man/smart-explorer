//! The pointer of a Wayland screen cast as PipeWire metadata (plan
//! 2026-11-14, package BK8a, B-16). With the portal's cursor mode `Metadata`
//! the pointer is not in the picture; every buffer may carry a
//! `SPA_META_Cursor` block with the pointer's position and, when the shape
//! changed, its bitmap. This module reads that block, keeps the pointer, and
//! hands it on in one of two ways (`PointerRoute`):
//! - the viewer draws the pointer itself (`cursor_overlay.rs` switched on by
//!   the daemon): shape and position go to `WorkerCursor`, the picture stays
//!   as captured, so the viewer shows exactly one pointer;
//! - the viewer cannot (version 0.5.196 has no `cursor_overlay`, or the
//!   session's source is a window family or a metadata channel): the pointer
//!   is mixed into the picture here, so it stays visible. A buffer without a
//!   new picture (the producer reports only that the pointer moved) republishes
//!   the last picture with the pointer at its new place.
//!
//! The block's layout and the pure parser are in `pipewire_cursor_meta.rs`.
use std::io;
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{
    lock_format, CallbackData, CaptureGrant, CaptureImageBuilder, CapturedImage, FrameLimits,
    FrameSize, PwBuffer,
};
use crate::screen::core::capture_admission::CaptureOwner;
use crate::screen::core::cursor_overlay::{self, CursorShape, WorkerCursor};

#[path = "pipewire_cursor_meta.rs"]
mod meta;
pub(super) use meta::request_pod;
use meta::{
    blend_premultiplied, parse_cursor_meta, BitmapUpdate, CursorMeta, PointerBitmap, CURSOR_STRUCT,
    SPA_META_CURSOR,
};

/// How the pointer reaches the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PointerRoute {
    /// The producer sends no pointer metadata: the pointer is in the picture
    /// (cursor mode Embedded) or absent (Hidden); nothing to do.
    InPicture,
    /// The viewer draws it: shape and position go to the overlay.
    Overlay,
    /// The viewer cannot: the pointer is mixed into the picture here.
    Blend,
}

/// The decision "overlay or mix in": `separated` is the overlay's state
/// (`WorkerCursor::separated`, on only when the viewer asked for it after
/// the host announced it and the capture said it can), `delivers` whether
/// the producer sent pointer metadata at all.
pub(super) fn route(separated: bool, delivers: bool) -> PointerRoute {
    match (delivers, separated) {
        (false, _) => PointerRoute::InPicture,
        (true, true) => PointerRoute::Overlay,
        (true, false) => PointerRoute::Blend,
    }
}

/// What the last blended picture showed: the pointer's shape and place.
type Signature = (u64, Option<(i32, i32)>);

/// The pointer as the producer last described it, and the last picture.
#[derive(Default)]
pub(super) struct Pointer {
    shape: Option<PointerBitmap>,
    /// Counts the shapes (a new bitmap, an empty one); names the shape
    /// towards the overlay.
    revision: u64,
    /// The hot spot; `None` while the pointer is hidden (id 0).
    at: Option<(i32, i32)>,
    delivers: bool,
    announced: bool,
    last_picture: Option<CapturedImage>,
    blended: Option<Signature>,
}

impl Pointer {
    /// Takes a parsed block; `true` when shape or place changed.
    pub(super) fn apply(&mut self, meta: CursorMeta) -> bool {
        let before = self.signature();
        if meta.id == 0 {
            self.at = None;
        } else {
            self.delivers = true;
            self.at = Some((meta.x, meta.y));
            match meta.bitmap {
                BitmapUpdate::Unchanged => {}
                BitmapUpdate::Empty => {
                    self.shape = None;
                    self.revision += 1;
                }
                BitmapUpdate::New {
                    width,
                    height,
                    rgba,
                } => {
                    self.shape = Some(PointerBitmap {
                        width,
                        height,
                        hot_x: meta.hot_x,
                        hot_y: meta.hot_y,
                        rgba,
                    });
                    self.revision += 1;
                }
            }
        }
        before != self.signature()
    }

    pub(super) fn delivers(&self) -> bool {
        self.delivers
    }

    fn signature(&self) -> Signature {
        (self.revision, self.at)
    }

    /// The shape and the hot spot when there is a pointer to show.
    pub(super) fn shown(&self) -> Option<(&PointerBitmap, (i32, i32))> {
        Some((self.shape.as_ref()?, self.at?))
    }

    /// Gives shape (when new) and position to the overlay.
    pub(super) fn publish_overlay(&self, worker: &WorkerCursor, area: FrameSize) {
        let serial = self.revision;
        if worker.shape_serial() != Some(serial) {
            let shape = self.shape.as_ref().and_then(|bitmap| {
                let id = worker.next_shape_id();
                CursorShape::sampled(
                    id,
                    bitmap.width,
                    bitmap.height,
                    bitmap.hot_x,
                    bitmap.hot_y,
                    |x, y| {
                        let at = ((y * bitmap.width + x) * 4) as usize;
                        [
                            bitmap.rgba[at],
                            bitmap.rgba[at + 1],
                            bitmap.rgba[at + 2],
                            bitmap.rgba[at + 3],
                        ]
                    },
                )
            });
            worker.publish_shape(serial, shape);
        }
        let (x, y) = self.at.unwrap_or((0, 0));
        let inside = self.at.is_some()
            && x >= 0
            && y >= 0
            && i64::from(x) < i64::from(area.width)
            && i64::from(y) < i64::from(area.height);
        worker.publish_position(x, y, area, inside);
    }

    /// `picture` with the pointer mixed in (a copy), or `picture` itself
    /// while there is no pointer to show.
    fn mixed_into(
        &self,
        picture: &CapturedImage,
        grant: &CaptureGrant,
    ) -> io::Result<CapturedImage> {
        let Some((bitmap, (x, y))) = self.shown() else {
            return Ok(picture.clone());
        };
        let size = picture.size();
        let mut copy = CaptureImageBuilder::new(
            size,
            FrameLimits::RESOURCE,
            grant,
            CaptureOwner::SourcePixels,
        )?;
        copy.copy_from(&picture.view())?;
        blend_premultiplied(copy.pixels_mut(), size, bitmap, x, y);
        Ok(copy.freeze())
    }
}

/// The pointer of one stream, touched by the stream's callbacks only.
#[derive(Default)]
pub(super) struct PointerFeed {
    inner: Mutex<Pointer>,
}

fn locked(inner: &Mutex<Pointer>) -> MutexGuard<'_, Pointer> {
    inner.lock().unwrap_or_else(PoisonError::into_inner)
}

impl PointerFeed {
    /// Forgets pointer and picture (the format changes or the stream ends).
    pub(super) fn reset(&self) {
        let mut pointer = locked(&self.inner);
        // The revision goes on: the overlay compares it with the shape it has.
        let revision = pointer.revision + 1;
        *pointer = Pointer {
            revision,
            ..Pointer::default()
        };
    }

    /// Whether the pointer is mixed into the picture here (viewer without
    /// overlay, producer with pointer metadata). The DMA-BUF capture (BK8b)
    /// then returns to shared memory, where this mixing happens.
    pub(super) fn blends(&self) -> bool {
        let delivers = locked(&self.inner).delivers();
        route(cursor_overlay::worker().separated(), delivers) == PointerRoute::Blend
    }

    /// The cursor block of `buffer`, read only after the same checks as the
    /// picture's copy (`copy_picture`): the buffer is one the stream added,
    /// and every range of its descriptors lies inside an admitted import.
    ///
    /// # Safety
    /// Called on the stream's loop thread with a dequeued `buffer`.
    unsafe fn read(data: &CallbackData, buffer: *mut PwBuffer) -> Option<CursorMeta> {
        if data.latest.import_blocked() {
            return None;
        }
        let grant = data.memory.current_grant()?;
        let known = grant
            .check_identity(data.memory.source(), data.memory.generation())
            .is_ok()
            && data
                .owners
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(buffer, grant.generation());
        if !known || data.imports.validate_buffer(buffer).is_err() {
            return None;
        }
        let spa = buffer.as_ref()?.buffer.as_ref()?;
        if spa.n_metas == 0 || spa.metas.is_null() {
            return None;
        }
        for meta in std::slice::from_raw_parts(spa.metas, spa.n_metas as usize) {
            if meta.type_ == SPA_META_CURSOR
                && meta.size as usize >= CURSOR_STRUCT
                && !meta.data.is_null()
            {
                let bytes = std::slice::from_raw_parts(meta.data.cast::<u8>(), meta.size as usize);
                return parse_cursor_meta(bytes);
            }
        }
        None
    }

    /// One dequeued buffer: reads its pointer block and returns what is to
    /// be published as the newest picture (`picture` is the buffer's copy,
    /// `None` for a buffer without a new picture).
    ///
    /// # Safety
    /// Called on the stream's loop thread with a dequeued `buffer`.
    pub(super) unsafe fn process(
        &self,
        data: &CallbackData,
        buffer: *mut PwBuffer,
        picture: Option<CapturedImage>,
    ) -> Option<CapturedImage> {
        let meta = Self::read(data, buffer);
        let Some((_, size)) = *lock_format(data) else {
            return picture;
        };
        let grant = data.memory.current_grant();
        self.step(
            cursor_overlay::worker(),
            size,
            meta,
            picture,
            grant.as_ref(),
        )
    }

    /// The decision of `process` without PipeWire.
    pub(super) fn step(
        &self,
        worker: &WorkerCursor,
        size: FrameSize,
        meta: Option<CursorMeta>,
        picture: Option<CapturedImage>,
        grant: Option<&CaptureGrant>,
    ) -> Option<CapturedImage> {
        let mut pointer = locked(&self.inner);
        let changed = meta.is_some_and(|meta| pointer.apply(meta));
        if pointer.delivers() && !pointer.announced {
            // This capture reads the pointer on its own now: the daemon's
            // wish for the overlay takes effect.
            pointer.announced = true;
            worker.set_supported();
        }
        if let Some(picture) = &picture {
            if pointer.delivers() {
                pointer.last_picture = Some(picture.clone());
            }
        }
        match route(worker.separated(), pointer.delivers()) {
            PointerRoute::InPicture => picture,
            PointerRoute::Overlay => {
                pointer.publish_overlay(worker, size);
                let raw = pointer.last_picture.clone();
                let was_blended = pointer.blended.take().is_some();
                match picture {
                    // A picture the viewer holds has the pointer in it: send a clean one.
                    None if was_blended => raw,
                    other => other,
                }
            }
            PointerRoute::Blend => {
                let signature = pointer.signature();
                let current = picture.is_some();
                if !current && !changed && pointer.blended == Some(signature) {
                    return None;
                }
                let source = picture.or_else(|| pointer.last_picture.clone())?;
                let mixed = match grant {
                    Some(grant) => pointer.mixed_into(&source, grant).unwrap_or(source),
                    None => source,
                };
                pointer.blended = Some(signature);
                Some(mixed)
            }
        }
    }
}

#[cfg(test)]
#[path = "pipewire_cursor_tests.rs"]
mod tests;
