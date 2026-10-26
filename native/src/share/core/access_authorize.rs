//! Capability decisions for proven principals (contract V7, E point 2):
//! which grant applies to a device that proved its relation, what it decides
//! for one capability at one moment, and which capability a start needs.
//! Exec checks every start here; the screen protocol of S4 every session.
use crate::share::access_grant::{Capability, CapabilityDecision, CapabilityState};
use crate::share::exec_policy::ExecGrant;
use crate::share::exec_types::ExecCommand;
use crate::share::profiles::fingerprint_matches;
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::types::{DirectGrantState, ShareAuthState};

/// The capability a start needs: commands for argv and shell starts, the
/// terminal for the login start.
pub(crate) fn start_capability(command: &ExecCommand) -> Capability {
    match command {
        ExecCommand::Argv { .. } | ExecCommand::Shell { .. } => Capability::ExecCommands,
        ExecCommand::Login => Capability::ExecTerminal,
    }
}

/// The grant of `principal` in `state`: the exact identity in an active
/// relation (Direct online and accepted, room joined and member admitted).
pub(crate) fn principal_grant<'a>(
    state: &'a ShareAuthState,
    principal: &ProvenPrincipal,
) -> Option<&'a ExecGrant> {
    let exact = |device_id: &str, public_key: &str, fingerprint: &str, node_id: &str| {
        device_id == principal.device_id
            && public_key == principal.public_key
            && fingerprint == principal.fingerprint
            && node_id == principal.node_id
            && fingerprint_matches(public_key, fingerprint)
    };
    match principal.relation_kind.as_str() {
        "direct" => {
            if principal.relation_id != state.identity.direct_lookup_id || !state.direct_online {
                return None;
            }
            state
                .direct_grants
                .iter()
                .find(|grant| {
                    grant.state == DirectGrantState::Accepted
                        && exact(
                            &grant.device_id,
                            &grant.public_key,
                            &grant.fingerprint,
                            &grant.node_id,
                        )
                })
                .map(|grant| &grant.exec)
        }
        "room" => state
            .rooms
            .iter()
            .find(|room| room.room_id == principal.relation_id && room.auto_join)?
            .members
            .iter()
            .find(|member| {
                member.is_admitted()
                    && exact(
                        &member.device_id,
                        &member.public_key,
                        &member.fingerprint,
                        &member.node_id,
                    )
            })
            .map(|member| &member.exec),
        _ => None,
    }
}

/// V7: the decision for `capability` of `principal` at `now` (Unix seconds,
/// host clock), with the revision and change time of its grant. Off, with
/// revision 0, when the principal has no grant in an active relation.
pub(crate) fn capability_for(
    state: &ShareAuthState,
    principal: &ProvenPrincipal,
    capability: Capability,
    now: i64,
) -> CapabilityState {
    principal_grant(state, principal).map_or_else(CapabilityState::default, |grant| {
        CapabilityState {
            decision: grant.decision_at(capability, now),
            policy_revision: grant.policy_revision,
            changed_at: grant.changed_at,
        }
    })
}

/// Whether `grant` admits an Exec connection at `now`: commands or terminal
/// is not off and the grant has not ended. Otherwise the handshake answers
/// `permission_denied`, which never tells why.
pub(crate) fn exec_connection_allowed(grant: &ExecGrant, now: i64) -> bool {
    let access = grant.access_grant();
    Capability::EXEC
        .into_iter()
        .any(|capability| access.decision_at(capability, now) != CapabilityDecision::Off)
}

/// Decision for a concrete host-resolved V2 request in exactly the proven
/// relation. Endpoint identity is derived from the authenticated state,
/// never from a display name or a union of room and Direct grants.
pub(crate) fn request_for(
    state: &ShareAuthState,
    principal: &ProvenPrincipal,
    request: &crate::share::access_constraints::AccessRequest,
    now: i64,
) -> CapabilityState {
    use crate::share::access_constraints::{AccessRequest, TcpDirection};
    let Some(grant) = principal_grant(state, principal) else {
        return CapabilityState::default();
    };
    let endpoints_match = match request {
        AccessRequest::TcpForward(request) => {
            let (initiator, target) = match request.direction {
                TcpDirection::PeerToHost => {
                    (principal.node_id.as_str(), state.identity.node_id.as_str())
                }
                TcpDirection::HostToPeer => {
                    (state.identity.node_id.as_str(), principal.node_id.as_str())
                }
            };
            request.initiator.as_str() == initiator && request.target.as_str() == target
        }
        AccessRequest::LoginAccess(_) => principal.relation_kind == "direct",
        AccessRequest::AppLaunch(_) => true,
    };
    CapabilityState {
        decision: if endpoints_match {
            grant.access_grant().decision_for(request, now)
        } else {
            CapabilityDecision::Off
        },
        policy_revision: grant.policy_revision,
        changed_at: grant.changed_at,
    }
}
