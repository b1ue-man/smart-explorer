//! Requests of one device share one shown surface (E28). The first open
//! request of a device is shown through `ConsentNotifier::show`; it is the
//! device's anchor. Every later request of that device joins the anchor's
//! surface without a second window or notification, and the window lists all
//! open requests of its device. Requests that equal each other (same device,
//! capability, exact target and grant revision) share one answer under „Immer
//! erlauben“ and „Ablehnen“. Showing and the anchors change only under
//! `Inner::gate`, which is taken before `Inner::pending` is locked. Only a
//! window or a notification is shared: a request answered `cli_only` shows
//! nothing, so the next request of its device asks again (a graphical session
//! may have appeared meanwhile).
use std::sync::{MutexGuard, PoisonError};

use super::{ApprovalId, ApprovalView, Inner, OpenApproval, Pending};
use crate::share::exec_access_view::ApprovalSurface;

/// The surface on which a device's requests are shown, and the id of the
/// request it was shown under: a window process is named after that id.
pub(super) struct Anchor {
    pub(super) surface: ApprovalSurface,
    pub(super) shown_as: ApprovalId,
}

/// A request that left the pending list, with what its removal changes.
pub(super) struct Ended {
    pub(super) id: ApprovalId,
    pub(super) device: String,
    pub(super) anchor: Option<Anchor>,
}

/// Whether later requests of a device join a request shown on `surface`.
pub(super) fn joinable(surface: ApprovalSurface) -> bool {
    matches!(
        surface,
        ApprovalSurface::Window | ApprovalSurface::Notification
    )
}

/// Whether `open` belongs to the device `device` (by its device id).
pub(super) fn of_device(device: &str, open: &OpenApproval) -> bool {
    open.principal.device_id == device
}

/// Equal requests: the same principal, capability, exact target (request,
/// command and account) and grant revision. Only equal requests share an answer.
pub(super) fn same_request(left: &OpenApproval, right: &OpenApproval) -> bool {
    left.principal == right.principal
        && left.capability == right.capability
        && left.policy_revision == right.policy_revision
        && left.request == right.request
        && left.view.command == right.view.command
        && left.view.account_label == right.view.account_label
}

impl Inner {
    /// Takes the gate under which showing and the anchors change.
    pub(super) fn showing(&self) -> MutexGuard<'_, ()> {
        self.gate.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The surface on which the device's requests are shown, if one is shown.
    pub(super) fn device_surface(&self, device: &str) -> Option<ApprovalSurface> {
        self.lock()
            .values()
            .filter(|entry| of_device(device, &entry.open))
            .find_map(|entry| entry.anchor.as_ref().map(|anchor| anchor.surface))
    }

    /// Finishes requests that were removed from the pending list: their
    /// notifications are withdrawn, and an anchor that ended passes on.
    pub(super) fn finish(&self, ended: Vec<Ended>) {
        for item in &ended {
            self.notifier().withdraw(&item.id);
        }
        for item in ended {
            if let Some(anchor) = item.anchor {
                self.hand_over(&item.device, anchor);
            }
        }
    }

    /// The anchor of `device` ended. Its surface passes to the oldest open
    /// request of the device. A window that is still open keeps showing the
    /// device's requests and is not opened again; a notification is shown
    /// again for the request it now belongs to.
    fn hand_over(&self, device: &str, anchor: Anchor) {
        if !matches!(anchor.surface, ApprovalSurface::Window) {
            self.show_again(device);
            return;
        }
        let Some((next, _)) = self.oldest_open(device) else {
            return;
        };
        if let Some(entry) = self.lock().get_mut(&next) {
            entry.open.view.surface = Some(anchor.surface.key().into());
            entry.anchor = Some(anchor);
        }
    }

    /// Shows the oldest open request of `device` again when no surface of the
    /// device lists its open requests; it becomes the device's anchor.
    fn show_again(&self, device: &str) {
        if self.device_surface(device).is_some() {
            return;
        }
        let Some((next, view)) = self.oldest_open(device) else {
            return;
        };
        let surface = self.notifier().show(&view);
        if let Some(entry) = self.lock().get_mut(&next) {
            entry.open.view.surface = surface.map(|surface| surface.key().into());
            entry.anchor = surface
                .filter(|surface| joinable(*surface))
                .map(|surface| Anchor {
                    surface,
                    shown_as: next.clone(),
                });
        }
    }

    /// The open request of `device` that was opened first (by time, then id).
    fn oldest_open(&self, device: &str) -> Option<(ApprovalId, ApprovalView)> {
        let pending = self.lock();
        let mut oldest: Option<(&ApprovalId, &Pending)> = None;
        for (id, entry) in pending.iter() {
            if !of_device(device, &entry.open) {
                continue;
            }
            let earlier = match oldest {
                None => true,
                Some((best_id, best)) => {
                    (entry.open.view.requested_at, id) < (best.open.view.requested_at, best_id)
                }
            };
            if earlier {
                oldest = Some((id, entry));
            }
        }
        oldest.map(|(id, entry)| (id.clone(), entry.open.view.clone()))
    }

    /// The process of the window shown under `window` has exited. Its anchor
    /// stops being the device's surface, so the device's next request opens a
    /// new window. Requests that are still open stay open. When the window's
    /// own request had ended, the window may never have listed them (it learns
    /// its device from that request), or they came after its last read: they
    /// are shown again, once for this exit. A window that ends while its own
    /// request is open (crash, update stop) shows nothing again.
    pub(super) fn end_window(&self, window: &ApprovalId) {
        let _showing = self.showing();
        let (device, own_open) = {
            let mut pending = self.lock();
            let mut device = None;
            for entry in pending.values_mut() {
                let ended = entry.anchor.as_ref().is_some_and(|anchor| {
                    anchor.shown_as == *window && matches!(anchor.surface, ApprovalSurface::Window)
                });
                if ended {
                    entry.anchor = None;
                    entry.open.view.surface = None;
                    device = Some(entry.open.principal.device_id.clone());
                }
            }
            (device, pending.contains_key(window))
        };
        if let Some(device) = device.filter(|_| !own_open) {
            self.show_again(&device);
        }
    }
}
