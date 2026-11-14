//! Confirmation of a start or session by the host user (PRM-M3, C5,
//! contract V7 point 6). A capability on `ask` makes every Exec start and
//! every screen session of S4 wait here until the host user answers
//! „Diesmal erlauben“ (`once`), „Immer erlauben“ (`always`) or „Ablehnen“
//! (`deny`), or until the requester withdraws it. There is no time limit
//! (C5, R-c). A request nobody can be shown, a withdrawn or narrowed grant and
//! a lost requester end it as refused, never as allowed. Neither a device
//! nor the host is limited in how many requests are open or Exec jobs run
//! at once (R15). The requests of one device share one shown window or
//! notification, and equal requests share one answer (E28, `group`).
//!
//! How the host user is asked is the platform's `ConsentNotifier`
//! (`consent_launch.rs`); this file has no platform code. S4 asks through
//! `ApprovalBroker::request(principal, capability, summary)`, the form of its
//! trait `ConsentBroker`, which U10 implements for the broker.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use tokio::sync::oneshot;

use crate::share::access_constraints::AccessRequest;
use crate::share::access_grant::Capability;
use crate::share::access_revocation::{self, AccessConfirmation};
use crate::share::exec_access_view::ApprovalSurface;
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::ExecGrantTarget;

#[path = "access_approval_view.rs"]
mod view;
pub use view::{ApprovalCommand, ApprovalDecision, ApprovalId, ApprovalSummary, ApprovalView};

#[path = "access_approval_group.rs"]
mod group;
use self::group::{joinable, same_request, Anchor, Ended};

const NOT_OPEN: &str = "the request is no longer open";

/// How this host shows a request to its user.
pub trait ConsentNotifier: Send + Sync {
    /// Shows `request`; `None` when nobody on this host can be asked.
    fn show(&self, request: &ApprovalView) -> Option<ApprovalSurface>;

    /// True only after this surface renders the full AccessV2 target.
    /// Legacy consent windows must never approve an invisible target.
    fn supports_contextual(&self) -> bool {
        false
    }

    /// The request ended (answered, withdrawn or cancelled).
    fn withdraw(&self, _id: &ApprovalId) {}
}

/// A host that cannot ask anybody (Android until the app sets its notifier).
pub struct NoConsent;

impl ConsentNotifier for NoConsent {
    fn show(&self, _request: &ApprovalView) -> Option<ApprovalSurface> {
        None
    }
}

/// How a request ended for the side that waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// „Diesmal erlauben“: exactly this start or session.
    Once,
    /// „Immer erlauben“: the capability of the grant is `allow` now.
    Always,
    /// „Ablehnen“, or the consent window was closed without an answer.
    Denied,
    /// Nobody on this host can be asked.
    Unavailable,
    /// The host app cannot show it: notifications are off (Android).
    Undeliverable,
    /// Ended without an answer: the request was withdrawn on this host.
    Withdrawn,
}

/// An open request as `Decide` with `always` needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenApproval {
    pub view: ApprovalView,
    pub principal: ProvenPrincipal,
    pub capability: Capability,
    pub policy_revision: u64,
    pub request: Option<AccessRequest>,
}

impl OpenApproval {
    /// The grant the request was made under, by its pinned identity.
    pub fn grant_target(&self) -> Result<ExecGrantTarget, String> {
        let principal = &self.principal;
        let (device_id, public_key, fingerprint, node_id) = (
            principal.device_id.clone(),
            principal.public_key.clone(),
            principal.fingerprint.clone(),
            principal.node_id.clone(),
        );
        match principal.relation_kind.as_str() {
            "direct" => Ok(ExecGrantTarget::Direct {
                device_id,
                public_key,
                fingerprint,
                node_id,
            }),
            "room" => Ok(ExecGrantTarget::RoomMember {
                room_id: principal.relation_id.clone(),
                device_id,
                public_key,
                fingerprint,
                node_id,
            }),
            other => Err(format!("unknown relation {other:?} of a request")),
        }
    }
}

struct Pending {
    open: OpenApproval,
    answer: oneshot::Sender<ApprovalOutcome>,
    /// Set on the request whose surface the requests of its device share.
    anchor: Option<Anchor>,
}

struct Inner {
    pending: Mutex<BTreeMap<ApprovalId, Pending>>,
    notifier: RwLock<Arc<dyn ConsentNotifier>>,
    /// Taken before `pending`: showing and the anchors change under it (`group`).
    gate: Mutex<()>,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, BTreeMap<ApprovalId, Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn notifier(&self) -> Arc<dyn ConsentNotifier> {
        self.notifier
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Ends an open request without an answer; its waiter sees `Withdrawn`.
    fn withdraw(&self, id: &ApprovalId) -> bool {
        let _showing = self.showing();
        let removed = self.lock().remove(id);
        let Some(Pending { open, anchor, .. }) = removed else {
            return false;
        };
        self.finish(vec![Ended {
            id: id.clone(),
            device: open.principal.device_id,
            anchor,
        }]);
        true
    }

    /// Answers `id` and, with `equal`, every open request equal to it: they
    /// share the answer (`group`). Returns how many requests other than `id`
    /// it answered. A waiter that is gone already refused on its own.
    fn answer(
        &self,
        id: &ApprovalId,
        outcome: ApprovalOutcome,
        equal: bool,
    ) -> Result<usize, String> {
        let _showing = self.showing();
        let ids: Vec<ApprovalId> = {
            let pending = self.lock();
            let target = pending
                .get(id)
                .map(|entry| entry.open.clone())
                .ok_or_else(|| NOT_OPEN.to_string())?;
            if equal {
                pending
                    .iter()
                    .filter(|(_, entry)| same_request(&entry.open, &target))
                    .map(|(key, _)| key.clone())
                    .collect()
            } else {
                vec![id.clone()]
            }
        };
        let mut ended = Vec::new();
        let mut also_answered = 0;
        {
            let mut pending = self.lock();
            for key in ids {
                if let Some(Pending {
                    open,
                    answer: waiter,
                    anchor,
                }) = pending.remove(&key)
                {
                    let _ = waiter.send(outcome);
                    if key != *id {
                        also_answered += 1;
                    }
                    ended.push(Ended {
                        id: key,
                        device: open.principal.device_id,
                        anchor,
                    });
                }
            }
        }
        self.finish(ended);
        Ok(also_answered)
    }
}

/// The confirmations of one process.
#[derive(Clone)]
pub struct ApprovalService {
    inner: Arc<Inner>,
}

impl ApprovalService {
    pub fn new(notifier: Arc<dyn ConsentNotifier>) -> Self {
        Self {
            inner: Arc::new(Inner {
                pending: Mutex::new(BTreeMap::new()),
                notifier: RwLock::new(notifier),
                gate: Mutex::new(()),
            }),
        }
    }

    /// How this host asks from now on (the Android app sets its own).
    #[allow(dead_code)] // Only the Android app sets one (`mobile`, Android builds only).
    pub fn set_notifier(&self, notifier: Arc<dyn ConsentNotifier>) {
        *self
            .inner
            .notifier
            .write()
            .unwrap_or_else(PoisonError::into_inner) = notifier;
    }

    /// The broker S4 asks.
    pub fn broker(&self) -> ApprovalBroker {
        ApprovalBroker {
            service: self.clone(),
        }
    }

    /// Opens a request and shows it. The ticket is answered at once when
    /// nobody can be asked. There is no limit on open requests (R15).
    pub fn request(
        &self,
        principal: &ProvenPrincipal,
        capability: Capability,
        summary: ApprovalSummary,
    ) -> ApprovalTicket {
        if capability.requires_context() {
            return self.answered(ApprovalId::unset(), ApprovalOutcome::Unavailable);
        }
        self.request_inner(principal, capability, summary, None)
    }

    /// Contextual requests carry their complete target into the human
    /// confirmation and bind a one-use receipt to this exact policy revision.
    pub fn request_access(
        &self,
        principal: &ProvenPrincipal,
        request: AccessRequest,
        summary: ApprovalSummary,
    ) -> ApprovalTicket {
        self.request_inner(principal, request.capability(), summary, Some(request))
    }

    fn request_inner(
        &self,
        principal: &ProvenPrincipal,
        capability: Capability,
        summary: ApprovalSummary,
        request: Option<AccessRequest>,
    ) -> ApprovalTicket {
        let notifier = self.inner.notifier();
        if request.is_some() && !notifier.supports_contextual() {
            return self.answered(ApprovalId::unset(), ApprovalOutcome::Unavailable);
        }
        let Ok(id) = ApprovalId::generate() else {
            return self.answered(ApprovalId::unset(), ApprovalOutcome::Unavailable);
        };
        let now = crate::share::core::now_secs();
        let mut view = ApprovalView::new(&id, principal, capability, &summary, now);
        view.request = request.clone();
        let (answer, receiver) = oneshot::channel();
        let open = OpenApproval {
            view: view.clone(),
            principal: principal.clone(),
            capability,
            policy_revision: summary.policy_revision,
            request: request.clone(),
        };
        // One showing at a time: a request of a device whose surface is shown
        // joins that surface instead of opening a second window (`group`).
        let _showing = self.inner.showing();
        let joined = self.inner.device_surface(&principal.device_id);
        self.inner.lock().insert(
            id.clone(),
            Pending {
                open,
                answer,
                anchor: None,
            },
        );
        // Outside the pending lock: showing may start the consent window.
        let surface = match joined {
            Some(surface) => surface,
            None => match notifier.show(&view) {
                Some(surface) => surface,
                None => {
                    self.inner.lock().remove(&id);
                    return self.answered(id, ApprovalOutcome::Unavailable);
                }
            },
        };
        if let Some(entry) = self.inner.lock().get_mut(&id) {
            entry.open.view.surface = Some(surface.key().into());
            if joined.is_none() && joinable(surface) {
                entry.anchor = Some(Anchor {
                    surface,
                    shown_as: id.clone(),
                });
            }
        }
        ApprovalTicket {
            id,
            surface: Some(surface),
            state: TicketState::Waiting(receiver),
            binding: request.map(|request| (principal.clone(), request, summary.policy_revision)),
            inner: self.inner.clone(),
        }
    }

    /// The open requests, oldest first.
    pub fn pending(&self) -> Vec<ApprovalView> {
        let mut views: Vec<ApprovalView> = self
            .inner
            .lock()
            .values()
            .map(|entry| entry.open.view.clone())
            .collect();
        views.sort_by(|left, right| {
            left.requested_at
                .cmp(&right.requested_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        views
    }

    pub fn open(&self, id: &ApprovalId) -> Option<OpenApproval> {
        self.inner.lock().get(id).map(|entry| entry.open.clone())
    }

    /// Answers the request `id`. „Immer erlauben“ and „Ablehnen“ also answer
    /// every open request equal to it (one shared answer, `group`); „Diesmal
    /// erlauben“ answers exactly `id`. Returns how many requests other than
    /// `id` were answered too (0 for „Diesmal erlauben“). „Immer erlauben“
    /// changes the grant first (the daemon's `Decide`); this only releases the
    /// waiters.
    pub fn decide(&self, id: &ApprovalId, decision: ApprovalDecision) -> Result<usize, String> {
        let outcome = match decision {
            ApprovalDecision::Once => ApprovalOutcome::Once,
            ApprovalDecision::Always => ApprovalOutcome::Always,
            ApprovalDecision::Deny => ApprovalOutcome::Denied,
        };
        self.inner
            .answer(id, outcome, decision != ApprovalDecision::Once)
    }

    /// The host app could not show `id` (Android without notifications).
    pub fn report_undeliverable(&self, id: &ApprovalId) -> Result<(), String> {
        self.inner
            .answer(id, ApprovalOutcome::Undeliverable, false)
            .map(|_| ())
    }

    /// The window shown under `window` has exited (`consent_launch`).
    pub fn window_ended(&self, window: &ApprovalId) {
        self.inner.end_window(window);
    }

    fn answered(&self, id: ApprovalId, outcome: ApprovalOutcome) -> ApprovalTicket {
        ApprovalTicket {
            id,
            surface: None,
            state: TicketState::Ready(outcome),
            binding: None,
            inner: self.inner.clone(),
        }
    }
}

enum TicketState {
    Ready(ApprovalOutcome),
    Waiting(oneshot::Receiver<ApprovalOutcome>),
}

/// The requester's handle of one request. Dropping it while the request is
/// open withdraws the request (the requester gave up).
pub struct ApprovalTicket {
    id: ApprovalId,
    surface: Option<ApprovalSurface>,
    state: TicketState,
    binding: Option<(ProvenPrincipal, AccessRequest, u64)>,
    inner: Arc<Inner>,
}

impl ApprovalTicket {
    /// Only an answered contextual ticket can mint a receipt, exactly once.
    pub fn take_confirmation(&mut self) -> Option<AccessConfirmation> {
        if !matches!(
            &self.state,
            TicketState::Ready(ApprovalOutcome::Once | ApprovalOutcome::Always)
        ) {
            return None;
        }
        self.binding.take().map(|(principal, request, revision)| {
            access_revocation::confirmation(principal, request, revision)
        })
    }

    pub fn id(&self) -> &ApprovalId {
        &self.id
    }

    /// Where the host user is asked; `None` when the ticket was answered at
    /// once.
    pub fn surface(&self) -> Option<ApprovalSurface> {
        self.surface
    }

    /// The outcome; waits for the host user while the request is open. The
    /// future can be dropped: the request then stays open for the ticket.
    pub async fn outcome(&mut self) -> ApprovalOutcome {
        let outcome = match &mut self.state {
            TicketState::Ready(outcome) => *outcome,
            TicketState::Waiting(receiver) => receiver.await.unwrap_or(ApprovalOutcome::Withdrawn),
        };
        self.state = TicketState::Ready(outcome);
        outcome
    }

    /// The requester (Exec client, screen viewer) cancels; `true` when the
    /// request was still open.
    pub fn cancel(mut self) -> bool {
        let open = matches!(self.state, TicketState::Waiting(_));
        self.state = TicketState::Ready(ApprovalOutcome::Withdrawn);
        open && self.inner.withdraw(&self.id)
    }
}

impl Drop for ApprovalTicket {
    fn drop(&mut self) {
        if matches!(self.state, TicketState::Waiting(_)) {
            self.inner.withdraw(&self.id);
        }
    }
}

/// The service as the screen strand asks it (V7: `ConsentBroker`).
#[derive(Clone)]
pub struct ApprovalBroker {
    service: ApprovalService,
}

impl ApprovalBroker {
    /// `ConsentBroker::request(principal, capability, summary) -> ApprovalTicket`.
    pub fn request(
        &self,
        principal: &ProvenPrincipal,
        capability: Capability,
        summary: ApprovalSummary,
    ) -> ApprovalTicket {
        self.service.request(principal, capability, summary)
    }
}

#[cfg(test)]
#[path = "access_approval_group_task_tests.rs"]
mod group_task_tests;
