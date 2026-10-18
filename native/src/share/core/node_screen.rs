//! The screen protocol on the share node (strand S4, SCR-M2): the ALPN
//! `smart-explorer/share-screen/1` beside FS, Exec and LAN link. The host side
//! writes `SERVER_HELLO` with a fresh challenge, proves the viewer's relation
//! with `prove_relation` (penalties and audit like Exec), binds the
//! connection to its principal (so removal and blocking close it) and hands
//! the session to `screen::core::host_server`. The node keeps the screen
//! registry; `invalidate_restrictions_at` ends the sessions a rights change
//! hits, and a timer ends those whose grant reaches its end
//! (`expired_restrictions`, `next_expiry`). The viewer side connects with
//! the screen ALPN (an older device refuses with `0x178`) and signs the
//! client hello with the relation secret.
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};
use std::time::Duration;

use iroh::endpoint::Connection;

use super::access_authorize::{capability_for, principal_grant};
use super::access_grant_runtime::{expired_restrictions, next_expiry};
use super::auth_penalty::PenaltyKind;
use super::core::{eio, now_secs, random_token};
use super::exec_audit::{self, AuditEvent};
use super::exec_registry::ExecRegistry;
use super::handshake_limits::ApplicationHandshakePermit;
use super::io_deadline;
use super::node::ShareIrohNode;
use super::relation_proof::{prove_relation, ProvenPrincipal};
use super::relation_rights::{RestrictionReason, RestrictionSet};
use super::service::ShareService;
use super::session::PeerPrincipal;
use super::types::ShareAuthState;
use crate::screen::core::authorize::{AuditSink, ScreenAuthority};
use crate::screen::core::host_backend::{ConsentBroker, ScreenHostBackend, SessionIndicator};
use crate::screen::core::host_server::{serve, HostContext, ProvenViewer};
use crate::screen::core::registry::{ScreenRegistry, SessionRestrictions, StopSelector};
use crate::screen::core::session_state::EndReason;
use crate::screen::core::wire::{recv_json, supports_version, CLIENT_HELLO};
use crate::screen::core::wire_hello::{
    send_hello_reply, send_server_hello, HelloReply, ScreenClientHello, ScreenErrorCode,
    ScreenHelloError, ScreenServerHello,
};
use crate::screen::os::indicator::DesktopIndicator;
use crate::screen::os::synthetic_backend::SyntheticBackend;
use crate::share::access_grant::{Capability, CapabilityState};

#[path = "node_screen_link.rs"]
mod link;
pub(crate) use link::ScreenLink;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
/// Longest sleep of the expiry timer; a moved grant end is read again then.
const EXPIRY_IDLE: Duration = Duration::from_secs(30);
/// Longest device name a session shows.
const MAX_NAME_CHARS: usize = 128;

fn ignore_entry(_: crate::share::access_audit::AccessEntry<'_>) {}

/// The screen part of one share node.
pub(crate) struct NodeScreen {
    registry: Arc<ScreenRegistry>,
    backend: RwLock<Arc<dyn ScreenHostBackend>>,
    indicator: RwLock<Arc<dyn SessionIndicator>>,
    broker: RwLock<Arc<dyn ConsentBroker>>,
    audit: RwLock<AuditSink>,
    inject_resets: Arc<AtomicU32>,
    expiry_running: AtomicBool,
}

fn read<T: Clone>(slot: &RwLock<T>) -> T {
    slot.read().unwrap_or_else(PoisonError::into_inner).clone()
}

impl NodeScreen {
    /// The test-picture host with the desktop indicator; open confirmations
    /// count against the Exec reservations of `exec_registry` (V7).
    pub(super) fn new(exec_registry: &Arc<ExecRegistry>) -> Self {
        let registry = Arc::new(ScreenRegistry::default());
        let gauge = exec_registry.clone();
        let broker = crate::share::consent_launch::approvals().broker(gauge);
        let (indicator, audit): (DesktopIndicator, AuditSink) = if cfg!(test) {
            (DesktopIndicator::with(None, false, &registry), ignore_entry)
        } else {
            let record: AuditSink = crate::share::access_audit::record_access;
            (DesktopIndicator::new(&registry), record)
        };
        Self {
            backend: RwLock::new(if cfg!(test) {
                Arc::new(SyntheticBackend) as Arc<dyn ScreenHostBackend>
            } else {
                crate::screen::os::host_backend()
            }),
            indicator: RwLock::new(Arc::new(indicator)),
            broker: RwLock::new(Arc::new(broker)),
            audit: RwLock::new(audit),
            inject_resets: Arc::new(AtomicU32::new(0)),
            expiry_running: AtomicBool::new(false),
            registry,
        }
    }

    pub(crate) fn registry(&self) -> &Arc<ScreenRegistry> {
        &self.registry
    }

    fn context(&self, auth: &Arc<Mutex<ShareAuthState>>) -> HostContext {
        HostContext {
            backend: read(&self.backend),
            registry: self.registry.clone(),
            authority: Arc::new(NodeAuthority { auth: auth.clone() }),
            broker: read(&self.broker),
            indicator: read(&self.indicator),
            audit: read(&self.audit),
            inject_resets: self.inject_resets.clone(),
        }
    }
}

/// The rights of the share host for the screen sessions.
struct NodeAuthority {
    auth: Arc<Mutex<ShareAuthState>>,
}

impl ScreenAuthority for NodeAuthority {
    fn capability(
        &self,
        principal: &ProvenPrincipal,
        capability: Capability,
        now: i64,
    ) -> Option<CapabilityState> {
        let state = self.auth.lock().ok()?;
        principal_grant(&state, principal)?;
        Some(capability_for(&state, principal, capability, now))
    }

    fn admit(
        &self,
        principal: &ProvenPrincipal,
        needed: &[Capability],
        now: i64,
        admit: &mut dyn FnMut(),
    ) -> bool {
        let Ok(state) = self.auth.lock() else {
            return false;
        };
        let allowed = principal_grant(&state, principal).is_some()
            && needed.iter().all(|capability| {
                capability_for(&state, principal, *capability, now)
                    .decision
                    .is_on()
            });
        if allowed {
            // Under the lock: a rights change cannot slip in between.
            admit();
        }
        allowed
    }

    fn stored_names(&self, principal: &ProvenPrincipal) -> (Option<String>, Option<String>) {
        let Ok(state) = self.auth.lock() else {
            return (None, None);
        };
        let same = |device_id: &str, public_key: &str| {
            device_id == principal.device_id && public_key == principal.public_key
        };
        match principal.relation_kind.as_str() {
            "direct" => {
                let grant = state
                    .direct_grants
                    .iter()
                    .find(|grant| same(&grant.device_id, &grant.public_key));
                (grant.map(|grant| grant.device_name.clone()), None)
            }
            "room" => {
                let room = state
                    .rooms
                    .iter()
                    .find(|room| room.room_id == principal.relation_id);
                let member = room.and_then(|room| {
                    room.members
                        .iter()
                        .find(|member| same(&member.device_id, &member.public_key))
                });
                (
                    member.map(|member| member.device_name.clone()),
                    room.map(|room| room.name.clone()),
                )
            }
            _ => (None, None),
        }
    }
}

impl SessionRestrictions for RestrictionSet {
    fn ends_session(&self, principal: &ProvenPrincipal, needs: &[Capability]) -> Option<EndReason> {
        let (kind, id) = (&principal.relation_kind, &principal.relation_id);
        let (key, node) = (&principal.public_key, &principal.node_id);
        let hit = needs
            .iter()
            .any(|capability| self.affects_capability(kind, id, key, node, *capability));
        if !hit {
            return None;
        }
        if self.everything_reason().is_some() {
            return Some(EndReason::HostShutdown);
        }
        let expired = self.items().iter().any(|item| {
            item.reason == RestrictionReason::GrantExpired && item.affects(kind, id, key, node)
        });
        Some(if expired {
            EndReason::GrantExpired
        } else {
            EndReason::CapabilityNarrowed
        })
    }
}

/// A device name as sessions show it: cut, without control characters.
fn shown_name(name: &str) -> String {
    let cut: String = name.chars().take(MAX_NAME_CHARS).collect();
    cut.chars()
        .map(|character| {
            if character.is_control() {
                '?'
            } else {
                character
            }
        })
        .collect()
}

impl ShareIrohNode {
    pub(crate) fn screen_registry(&self) -> &Arc<ScreenRegistry> {
        self.screen.registry()
    }

    /// Host side of one connection with the screen ALPN.
    pub(super) async fn handle_screen_connection(
        self: Arc<Self>,
        connection: Connection,
        permit: ApplicationHandshakePermit,
    ) -> io::Result<()> {
        let _incoming = self.track_incoming(&connection)?;
        self.require_sharing_active()?;
        let remote = connection.remote_id().to_string();
        let deadline = tokio::time::Instant::now() + HANDSHAKE_TIMEOUT;
        let opened = tokio::time::timeout_at(deadline, connection.open_bi()).await;
        let (mut send, mut recv) = opened
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "screen handshake timeout"))?
            .map_err(eio)?;
        let identity = self
            .auth
            .lock()
            .map_err(|_| eio("Share screen authorization state is locked"))?
            .identity
            .clone();
        let server = ScreenServerHello::new(
            random_token(32).map_err(eio)?,
            identity.device_id,
            identity.public_key,
            identity.fingerprint,
            identity.node_id,
        );
        let hello = send_server_hello(&mut send, &server);
        io_deadline::run_until(deadline, "screen hello timeout", hello).await?;
        let received = recv_json::<_, ScreenClientHello>(&mut recv, CLIENT_HELLO);
        let client = match io_deadline::run_until(deadline, "screen hello timeout", received).await
        {
            Ok(client) => client,
            Err(error) => {
                let now = std::time::Instant::now();
                self.auth_penalties()
                    .record(&remote, PenaltyKind::NoAuthentication, now);
                return Err(error);
            }
        };
        let refuse = |code| HelloReply::Refused(ScreenHelloError::new(code));
        if !supports_version(client.protocol_version) {
            let reply = refuse(ScreenErrorCode::ScreenUnsupported);
            send_hello_reply(&mut send, &reply).await?;
            let _ = send.finish();
            return Ok(());
        }
        let proven = match client.validate() {
            Ok(()) => {
                let state = self
                    .auth
                    .lock()
                    .map_err(|_| eio("Share screen authorization state is locked"))?;
                prove_relation(&server.relation(), &client.relation(), &remote, &state)
                    .map_err(Some)
            }
            Err(_) => Err(None),
        };
        let principal = match proven {
            Ok(principal) => principal,
            Err(rejection) => {
                let failed =
                    rejection.is_none_or(|rejection| rejection.counts_as_failed_authentication());
                if failed {
                    let now = std::time::Instant::now();
                    self.auth_penalties()
                        .record(&remote, PenaltyKind::AuthFailure, now);
                }
                exec_audit::record(AuditEvent::AuthRejected {
                    remote_node: &remote,
                    claimed_device: &client.device_id,
                    reason: if failed {
                        "screen: authentication failed"
                    } else {
                        "screen: relation not active"
                    },
                });
                send_hello_reply(&mut send, &refuse(ScreenErrorCode::PermissionDenied)).await?;
                let _ = send.finish();
                let _ = tokio::time::timeout(Duration::from_secs(2), connection.closed()).await;
                return Ok(());
            }
        };
        let peer = PeerPrincipal::new(
            principal.relation_kind.clone(),
            principal.relation_id.clone(),
            principal.device_id.clone(),
            principal.public_key.clone(),
            principal.node_id.clone(),
        );
        self.bind_incoming_principal(&connection, peer).await?;
        // A verified hello ends the handshake deadline; a confirmation by the
        // host user may take as long as it takes (C5).
        drop(permit);
        let context = self.screen.context(&self.auth);
        let stored = context.authority.stored_names(&principal).0;
        let device_name = shown_name(stored.as_deref().unwrap_or(&principal.device_name));
        self.ensure_screen_expiry();
        let viewer = ProvenViewer {
            connection,
            send,
            recv,
            principal,
            device_name,
            hello: client,
        };
        serve(context, viewer).await
    }

    /// Starts the expiry timer of the screen sessions unless it runs: it
    /// sleeps until the next grant end (at most `EXPIRY_IDLE`, or until the
    /// sessions change) and ends the sessions of every grant that ended.
    fn ensure_screen_expiry(self: &Arc<Self>) {
        if self.screen.expiry_running.swap(true, Ordering::AcqRel) {
            return;
        }
        let node = Arc::downgrade(self);
        self.rt.spawn(screen_expiry(node));
    }
}

async fn screen_expiry(node: Weak<ShareIrohNode>) {
    // A grant that ends in the very second the timer starts counts as well.
    let mut since = now_secs().saturating_sub(1);
    loop {
        let Some(current) = node.upgrade() else {
            return;
        };
        let registry = current.screen.registry.clone();
        let now = now_secs();
        let next = match current.auth.lock() {
            Ok(state) => next_expiry(&state, now),
            Err(_) => return,
        };
        drop(current);
        let wait = next.map_or(EXPIRY_IDLE, |end| {
            Duration::from_secs(u64::try_from(end.saturating_sub(now)).unwrap_or(0))
        });
        tokio::select! {
            () = tokio::time::sleep(wait.min(EXPIRY_IDLE)) => {}
            () = registry.changed() => {}
        }
        let Some(current) = node.upgrade() else {
            return;
        };
        let now = now_secs();
        if let Ok(state) = current.auth.lock() {
            let restrictions = expired_restrictions(&state, since, now);
            if !restrictions.is_empty() {
                registry.restrict_authorization(state.authorization_epoch, &restrictions);
            }
        }
        since = now;
        if registry.is_empty() {
            let running = &current.screen.expiry_running;
            running.store(false, Ordering::Release);
            // A session that registered meanwhile keeps the timer.
            if registry.is_empty() || running.swap(true, Ordering::AcqRel) {
                return;
            }
        }
    }
}

impl ShareService {
    /// The screen sessions this device hosts.
    pub(crate) fn screen_registry(&self) -> Arc<ScreenRegistry> {
        self.iroh.screen_registry().clone()
    }

    /// Ends the selected sessions of this host (`se screen stop`, the tab,
    /// the indicator).
    pub(crate) fn stop_screen_sessions(&self, selector: &StopSelector) -> usize {
        self.iroh
            .screen_registry()
            .stop(selector, EndReason::HostStopped)
    }
}

/// Stops every screen session of the node (sharing stops).
pub(super) fn stop_all(screen: &NodeScreen) {
    screen
        .registry
        .stop(&StopSelector::All, EndReason::HostShutdown);
}

#[cfg(test)]
#[path = "node_screen_tests.rs"]
mod tests;
