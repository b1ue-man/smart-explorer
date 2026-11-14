//! The daemon's and the viewer's side of the pointer overlay (plan
//! 2026-11-14, package P6): `CursorFeed` holds the newest pointer of a host
//! process for the session that sends it on, `CursorView` the pointer a
//! viewer draws.
use std::sync::{Arc, Mutex};

use super::{locked, CursorPosition, CursorShape, CursorUpdate};

#[derive(Default)]
struct FeedState {
    shape: Option<CursorShape>,
    shape_revision: u64,
    position: Option<CursorPosition>,
    position_revision: u64,
}

/// The newest pointer of a host process as the daemon holds it. The reader
/// thread of the host process publishes into it and wakes the session, which
/// sends the change on at once (the host process sends at most one position
/// per picture, so the viewer gets at most one per picture tick).
#[derive(Default)]
pub struct CursorFeed {
    state: Mutex<FeedState>,
    woken: tokio::sync::Notify,
}

/// What a reader of a `CursorFeed` has taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CursorSeen {
    shape: u64,
    position: u64,
}

impl CursorFeed {
    pub fn publish(&self, update: CursorUpdate) {
        {
            let mut state = locked(&self.state);
            match update {
                CursorUpdate::Shape(shape) => {
                    state.shape = Some(shape);
                    state.shape_revision += 1;
                }
                CursorUpdate::Position(position) => {
                    state.position = Some(position);
                    state.position_revision += 1;
                }
            }
        }
        self.woken.notify_one();
    }

    /// Resolves once something was published since the last wait; a
    /// publication between two waits is kept for the next one.
    pub async fn changed(&self) {
        self.woken.notified().await;
    }

    /// Forgets the held pointer (the overlay starts): the host process sends
    /// shape and position again.
    pub fn reset(&self) {
        let mut state = locked(&self.state);
        state.shape = None;
        state.position = None;
    }

    /// The changes since `seen`: the shape first, then the position.
    pub fn poll(&self, seen: &mut CursorSeen) -> Vec<CursorUpdate> {
        let state = locked(&self.state);
        let mut updates = Vec::new();
        if state.shape_revision != seen.shape {
            seen.shape = state.shape_revision;
            if let Some(shape) = &state.shape {
                updates.push(CursorUpdate::Shape(shape.clone()));
            }
        }
        if state.position_revision != seen.position {
            seen.position = state.position_revision;
            if let Some(position) = state.position {
                updates.push(CursorUpdate::Position(position));
            }
        }
        updates
    }
}

/// The pointer as a viewer holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CursorView {
    pub shape: Option<Arc<CursorShape>>,
    pub position: Option<CursorPosition>,
}

impl CursorView {
    pub fn set_shape(&mut self, shape: CursorShape) {
        self.shape = Some(Arc::new(shape));
    }

    pub fn set_position(&mut self, position: CursorPosition) {
        self.position = Some(position);
    }

    /// The pointer to draw: a visible position whose shape is the one held.
    pub fn visible(&self) -> Option<(&CursorShape, &CursorPosition)> {
        let shape = self.shape.as_deref()?;
        let position = self.position.as_ref()?;
        (position.visible && position.shape_id == shape.id).then_some((shape, position))
    }
}
