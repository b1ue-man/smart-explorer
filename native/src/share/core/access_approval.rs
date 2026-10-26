//! Confirmation of a start or session by the host user (PRM-M3, C5,
//! contract V7 point 6). A capability on `ask` makes every Exec start and
//! every screen session of S4 wait here until the host user answers
//! „Diesmal erlauben“ (`once`), „Immer erlauben“ (`always`) or „Ablehnen“
//! (`deny`), or until the requester withdraws it. There is no time limit
//! (C5, R-c). A request nobody can be shown, a withdrawn or narrowed grant and
//! a lost requester end it as refused, never as allowed. Neither a device
//! nor the host is limited in how many requests are open or Exec jobs run
//! at once (R15).
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
pub(crate) use view::{
    ApprovalCommand, ApprovalDecision, ApprovalId, ApprovalSummary, ApprovalView,
};

const NOT_OPEN: &str = "the request is no longer open";

/// How this host shows a request to its user.
pub(crate) trait ConsentNotifier: Send + Sync {
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
pub(crate) struct NoConsent;

impl ConsentNotifier for NoConsent {
    fn show(&self, _request: &ApprovalView) -> Option<ApprovalSurface> {
        None
    }
}

/// How a request ended for the side that waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApprovalOutcome {
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
pub(crate) struct OpenApproval {
    pub(crate) view: ApprovalView,
    pub(crate) principal: ProvenPrincipal,
    pub(crate) capability: Capability,
    pub(crate) policy_revision: u64,
    pub(crate) request: Option<AccessRequest>,
}

impl OpenApproval {
    /// The grant the request was made under, by its pinned identity.
    pub(crate) fn grant_target(&self) -> Result<ExecGrantTarget, String> {
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
}

struct Inner {
    pending: Mutex<BTreeMap<ApprovalId, Pending>>,
    notifier: RwLock<Arc<dyn ConsentNotifier>>,
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
        let removed = self.lock().remove(id).is_some();
        if removed {
            self.notifier().withdraw(id);
        }
        removed
    }

    fn answer(&self, id: &ApprovalId, outcome: ApprovalOutcome) -> Result<(), String> {
        let entry = self.lock().remove(id).ok_or_else(|| NOT_OPEN.to_string())?;
        // A waiter that is gone already refused on its own.
        let _ = entry.answer.send(outcome);
        self.notifier().withdraw(id);
        Ok(())
    }
}

/// The confirmations of one process.
#[derive(Clone)]
pub(crate) struct ApprovalService {
    inner: Arc<Inner>,
}

impl ApprovalService {
    pub(crate) fn new(notifier: Arc<dyn ConsentNotifier>) -> Self {
        Self {
            inner: Arc::new(Inner {
                pending: Mutex::new(BTreeMap::new()),
                notifier: RwLock::new(notifier),
            }),
        }
    }

    /// How this host asks from now on (the Android app sets its own).
    #[allow(dead_code)] // Only the Android app sets one (`mobile`, Android builds only).
    pub(crate) fn set_notifier(&self, notifier: Arc<dyn ConsentNotifier>) {
        *self
            .inner
            .notifier
            .write()
            .unwrap_or_else(PoisonError::into_inner) = notifier;
    }

    /// The broker S4 asks.
    pub(crate) fn broker(&self) -> ApprovalBroker {
        ApprovalBroker {
            service: self.clone(),
        }
    }

    /// Opens a request and shows it. The ticket is answered at once when
    /// nobody can be asked. There is no limit on open requests (R15).
    pub(crate) fn request(
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
    pub(crate) fn request_access(
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
        self.inner
            .lock()
            .insert(id.clone(), Pending { open, answer });
        // Outside the lock: showing may start the consent window.
        let Some(surface) = notifier.show(&view) else {
            self.inner.lock().remove(&id);
            return self.answered(id, ApprovalOutcome::Unavailable);
        };
        if let Some(entry) = self.inner.lock().get_mut(&id) {
            entry.open.view.surface = Some(surface.key().into());
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
    pub(crate) fn pending(&self) -> Vec<ApprovalView> {
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

    pub(crate) fn open(&self, id: &ApprovalId) -> Option<OpenApproval> {
        self.inner.lock().get(id).map(|entry| entry.open.clone())
    }

    /// Answers exactly the request `id`. „Immer erlauben“ changes the grant
    /// first (the daemon's `Decide`); this only releases the waiter.
    pub(crate) fn decide(&self, id: &ApprovalId, decision: ApprovalDecision) -> Result<(), String> {
        let outcome = match decision {
            ApprovalDecision::Once => ApprovalOutcome::Once,
            ApprovalDecision::Always => ApprovalOutcome::Always,
            ApprovalDecision::Deny => ApprovalOutcome::Denied,
        };
        self.inner.answer(id, outcome)
    }

    /// The host app could not show `id` (Android without notifications).
    pub(crate) fn report_undeliverable(&self, id: &ApprovalId) -> Result<(), String> {
        self.inner.answer(id, ApprovalOutcome::Undeliverable)
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
pub(crate) struct ApprovalTicket {
    id: ApprovalId,
    surface: Option<ApprovalSurface>,
    state: TicketState,
    binding: Option<(ProvenPrincipal, AccessRequest, u64)>,
    inner: Arc<Inner>,
}

impl ApprovalTicket {
    /// Only an answered contextual ticket can mint a receipt, exactly once.
    pub(crate) fn take_confirmation(&mut self) -> Option<AccessConfirmation> {
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

    pub(crate) fn id(&self) -> &ApprovalId {
        &self.id
    }

    /// Where the host user is asked; `None` when the ticket was answered at
    /// once.
    pub(crate) fn surface(&self) -> Option<ApprovalSurface> {
        self.surface
    }

    /// The outcome; waits for the host user while the request is open. The
    /// future can be dropped: the request then stays open for the ticket.
    pub(crate) async fn outcome(&mut self) -> ApprovalOutcome {
        let outcome = match &mut self.state {
            TicketState::Ready(outcome) => *outcome,
            TicketState::Waiting(receiver) => receiver.await.unwrap_or(ApprovalOutcome::Withdrawn),
        };
        self.state = TicketState::Ready(outcome);
        outcome
    }

    /// The requester (Exec client, screen viewer) cancels; `true` when the
    /// request was still open.
    pub(crate) fn cancel(mut self) -> bool {
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
pub(crate) struct ApprovalBroker {
    service: ApprovalService,
}

impl ApprovalBroker {
    /// `ConsentBroker::request(principal, capability, summary) -> ApprovalTicket`.
    pub(crate) fn request(
        &self,
        principal: &ProvenPrincipal,
        capability: Capability,
        summary: ApprovalSummary,
    ) -> ApprovalTicket {
        self.service.request(principal, capability, summary)
    }
}
