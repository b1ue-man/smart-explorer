//! The host user's indicator and the consent broker of the screen host
//! (C7, contract V7), split from `host_backend.rs` to keep it small.
use crate::share::access_approval::{ApprovalBroker, ApprovalSummary, ApprovalTicket};
use crate::share::access_grant::Capability;
use crate::share::relation_proof::ProvenPrincipal;

/// The host user's sign that the screen is watched (C7): the window
/// `smart_explorer --screen-indicator` on desktops.
pub trait SessionIndicator: Send + Sync {
    /// Whether the indicator can be shown at all (desktop app present).
    fn available(&self) -> bool;
    /// Shows it, or keeps it shown; an error ends the session with
    /// `indicator_unavailable`.
    fn show(&self) -> Result<(), String>;
}

/// The screen strand's view of the S1 confirmation service (V7).
pub trait ConsentBroker: Send + Sync {
    /// Opens a request for `capability`; the ticket answers at once when
    /// nobody can be asked.
    fn request(
        &self,
        principal: &ProvenPrincipal,
        capability: Capability,
        summary: ApprovalSummary,
    ) -> ApprovalTicket;
}

impl ConsentBroker for ApprovalBroker {
    fn request(
        &self,
        principal: &ProvenPrincipal,
        capability: Capability,
        summary: ApprovalSummary,
    ) -> ApprovalTicket {
        ApprovalBroker::request(self, principal, capability, summary)
    }
}
