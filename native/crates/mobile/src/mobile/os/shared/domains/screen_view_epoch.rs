//! One viewer session over several connections of the relay (review
//! 2026-11-19, Gemini 3/9.4): when the daemon rebuilds a host session
//! silently, it hands the session a second `Accepted`, and the new host
//! session counts its group and frame ids from 1 again. The bridge keeps the
//! group ids the app sees rising (it adds the highest id of the connections
//! before) and gives the host its own ids back; the app starts a new picture
//! epoch at the second `accepted` event (`ScreenSession.kt`). The requested
//! key frame of the connection before no longer holds a new request back.
use super::*;
use crate::screen::core::wire_control::KeyframeRequest;

impl ViewSession {
    /// The relay accepted a connection: from the second one on, the group
    /// ids of the new host session follow the highest one seen so far.
    pub(super) fn begin_connection(&self) {
        if self.connections.fetch_add(1, Ordering::SeqCst) == 0 {
            return;
        }
        self.gop_offset
            .store(self.latest_gop.load(Ordering::SeqCst), Ordering::SeqCst);
        *lock(&self.keyframes) = KeyframeRequester::default();
    }

    /// The id the app sees for group `host` of the current connection.
    pub(super) fn viewer_gop(&self, host: u64) -> u64 {
        host.saturating_add(self.gop_offset.load(Ordering::SeqCst))
    }

    /// `frame` with the host's own group id, `None` for a frame that names a
    /// group of a connection before.
    fn to_host(&self, frame: &ViewerFrame) -> Option<ViewerFrame> {
        let offset = self.gop_offset.load(Ordering::SeqCst);
        let host = |gop: u64| (gop > offset).then(|| gop - offset);
        Some(match frame {
            ViewerFrame::Presented { gop_id, frame_id } => ViewerFrame::Presented {
                gop_id: host(*gop_id)?,
                frame_id: *frame_id,
            },
            ViewerFrame::KeyframeRequest(request) => {
                ViewerFrame::KeyframeRequest(KeyframeRequest {
                    reason: request.reason,
                    gop_id: request.gop_id.and_then(host),
                })
            }
            other => other.clone(),
        })
    }

    pub(in super::super) fn send(&self, frame: &ViewerFrame) -> Result<(), ApiError> {
        if lock(&self.family).is_some() && matches!(frame, ViewerFrame::Input(_)) {
            return Err(ApiError::new(
                "permission",
                "Familieneingabe benötigt Snapshot und Fokusbestätigung.",
            ));
        }
        if let ViewerFrame::Presented { gop_id, .. } = frame {
            if lock(&self.family).as_ref().is_some_and(|family| {
                family.snapshot.as_ref().is_none_or(|snapshot| {
                    *lock(&self.family_picture) != Some((*gop_id, snapshot.layout_revision))
                })
            }) {
                return Ok(());
            }
        }
        let send = lock(&self.link)
            .as_ref()
            .map(|link| link.send.clone())
            .ok_or_else(|| ApiError::new("busy", "Die Sitzung ist noch nicht verbunden."))?;
        // The host knows its own group ids (`to_host`); a frame of a group of
        // the connection before goes nowhere.
        let Some(outgoing) = self.to_host(frame) else {
            return Ok(());
        };
        send(&outgoing).map_err(|error| ApiError::new("network", error.to_string()))?;
        if let ViewerFrame::Presented { gop_id, .. } = frame {
            if lock(&self.family).as_ref().is_some_and(|family| {
                family.snapshot.as_ref().is_some_and(|snapshot| {
                    *lock(&self.family_picture) == Some((*gop_id, snapshot.layout_revision))
                })
            }) {
                self.family_presented.store(true, Ordering::SeqCst);
            }
        }
        Ok(())
    }
}
