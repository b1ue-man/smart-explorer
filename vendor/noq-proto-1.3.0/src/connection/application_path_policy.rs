// Smart Explorer P28b/V3 patch (2026-10-25); see vendor/SMART_EXPLORER_ROUTE_POLICY.md.
//! Opt-in, connection-local admission at the QUIC serialization boundary.

use std::collections::{BTreeMap, BTreeSet};

use super::{Connection, PathId, timer::{ConnTimer, PathTimer, Timer}};
use crate::{Duration, Instant, n0_nat_traversal};

/// An opaque observation of one path in one policy revision.
///
/// Tokens belong to the originating connection. Obtain a fresh token after every
/// policy mutation. A token never authorizes a different incarnation of a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplicationPathToken {
    connection: u64,
    generation: u64,
    revision: u64,
    path_id: PathId,
    path_generation: u64,
}

impl ApplicationPathToken {
    /// QUIC path identifier, scoped to the originating connection.
    pub fn path_id(self) -> PathId { self.path_id }
    /// Caller-controlled network/policy generation.
    pub fn generation(self) -> u64 { self.generation }
    /// Incarnation of the path, including RFC 9000 migrations.
    pub fn path_generation(self) -> u64 { self.path_generation }
}

/// One explicit mutation of an enabled application path policy.
#[derive(Debug, Clone, Copy)]
pub enum ApplicationPathUpdate {
    /// Start a strictly newer generation, revoking all prior admission.
    BeginGeneration(u64),
    /// Revoke admission in this generation, invalidating previously observed tokens.
    Block(u64),
    /// Admit exactly the observed, validated path. All previous admission is revoked.
    Allow(ApplicationPathToken),
    /// Retire an unadmitted, nonvalidating redundant path for another candidate.
    /// The token and all protection conditions are rechecked atomically.
    RetireRedundantPath(ApplicationPathToken),
}

/// A rejected policy update. No application contents are included in errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ApplicationPathPolicyError {
    /// Policy was not enabled before the connection was created.
    #[error("application path policy is disabled")]
    Disabled,
    /// Noq observed a network change; BeginGeneration is required before admission.
    #[error("a new application path policy generation is required after network change")]
    NetworkGenerationRequired,
    /// The connection has closed.
    #[error("connection is closed")]
    Closed,
    /// Generation must match, or strictly increase for BeginGeneration.
    #[error("stale application path policy generation")]
    StaleGeneration,
    /// Token belongs to a different connection or an obsolete policy revision/path.
    #[error("stale application path observation")]
    StaleObservation,
    /// The handshake/path validation has not completed, or the path was abandoned.
    #[error("application path is not validated")]
    UnvalidatedPath,
    /// Capacity or the current grant/validation/survivor state prevents retirement.
    #[error("application path is protected from retirement")]
    ProtectedPath,
    /// The policy revision cannot advance without wrapping.
    #[error("application path policy revision exhausted")]
    RevisionExhausted,
}

/// Current facts for one QUIC path. This does not classify LAN or authorize a peer.
#[derive(Debug, Clone)]
pub struct ApplicationPathObservation {
    /// Token usable with Allow while this observation is current.
    pub token: ApplicationPathToken,
    /// Transport tuple in Noq socket coordinates; Iroh relay addresses may be mapped.
    /// Use the Iroh paths() view to resolve established PathIds to transport addresses.
    pub network_path: crate::FourTuple,
    /// Whether the path has the current required proof. Allow also checks the
    /// snapshot generation/revision and requires_new_generation flag.
    pub validated: bool,
    /// Whether this policy gate admits application frames on this path. A disabled
    /// policy imposes no restriction; ordinary transport scheduling still applies.
    pub data_allowed: bool,
    /// Whether the negotiated path capacity is full and this unadmitted path
    /// can be retired without removing a validation or the last usable path.
    /// RetireRedundantPath rechecks this under the connection's mutation lock.
    pub retirement_allowed: bool,
    /// Whether Noq is still validating this path with on-path challenges.
    pub validation_in_progress: bool,
    /// Whether a previous RFC 9000 incarnation is still being validated.
    pub previous_validation_in_progress: bool,
    /// Actual Noq validation-failure timer, if armed.
    pub validation_deadline: Option<Instant>,
    /// Actual Noq challenge-retry timer, if armed.
    pub challenge_retry_deadline: Option<Instant>,
    /// Serialized STREAM/DATAGRAM payload bytes, including retransmits and dropped I/O plans.
    pub application_tx_bytes: u64,
    /// Authenticated processed payload bytes, including repeated STREAM ranges.
    pub application_rx_bytes: u64,
    /// Current PTO, including the negotiated maximum ACK delay.
    pub pto: Duration,
}

/// A connection-local snapshot. Observation alone never grants application sending.
#[derive(Debug, Clone)]
pub struct ApplicationPathPolicySnapshot {
    /// Set only by transport configuration before connection construction.
    pub enabled: bool,
    /// Monotone generation controlled by the app adapter.
    pub generation: u64,
    /// Revision invalidating tokens and retained encrypted transmits.
    pub revision: u64,
    /// A transport-observed network change requires BeginGeneration before Allow.
    pub requires_new_generation: bool,
    /// A confirmed change is still waiting for transport readiness. Route
    /// collection must wait; completion requires a fresh generation afterwards.
    pub network_change_in_progress: bool,
    /// Number of transport-observed network changes in this connection.
    pub network_change_count: u64,
    /// Monotone connection-lifetime serialized payload bytes, including retransmits
    /// and subsequently discarded Pending I/O plans. This is not acknowledged goodput.
    pub application_tx_bytes: u64,
    /// Monotone authenticated processed payload bytes, including repeated STREAM ranges.
    /// Both counters exclude UDP/QUIC/crypto headers and transport control frames.
    pub application_rx_bytes: u64,
    /// All current paths, including unvalidated and abandoned paths.
    pub paths: Vec<ApplicationPathObservation>,
    /// NAT retry/probe facts for the current Noq round, if negotiated.
    pub nat_round: Option<n0_nat_traversal::RoundObservation>,
    /// Actual connection-wide NAT retry timer, if armed.
    pub nat_retry_deadline: Option<Instant>,
    /// Initial configured RTT used by the NAT retry implementation.
    pub initial_rtt: Duration,
    /// Initial PTO including maximum ACK delay.
    pub initial_pto: Duration,
    /// PTO of the best currently validated path, or the initial PTO.
    pub best_validated_pto: Duration,
}

/// Stamp for a retained encrypted transmit. Compare again before retrying I/O.
///
/// This is an integration boundary for Noq's serialized connection driver;
/// applications should use the policy snapshot and update API instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplicationTransmitStamp {
    connection: u64,
    generation: u64,
    revision: u64,
    path_generation_counter: u64,
    admitted: Option<(PathId, u64)>,
}

#[derive(Debug, Default)]
pub(super) struct ApplicationPathPolicy {
    pub(super) generation: u64,
    pub(super) revision: u64,
    pub(super) admitted: Option<(PathId, u64)>,
    pub(super) connection: u64,
    pub(super) pending_validation: BTreeSet<PathId>,
    pub(super) requires_new_generation: bool,
    network_change_count: u64,
    pending_network_change_pto: Option<Duration>,
    application_tx_bytes: u64,
    application_rx_bytes: u64,
    pub(super) path_payload_bytes: BTreeMap<PathId, (u64, u64)>,
}

impl ApplicationPathPolicy {
    /// Bind default policy state to the connection's already generated identity.
    pub(super) fn new(connection: u64) -> Self {
        Self {
            connection,
            ..Self::default()
        }
    }
}

impl Connection {
    /// Observe current policy, NAT-round and path-validation facts atomically.
    ///
    /// Times use the same monotone clock as this Connection. The app owns a finite
    /// candidate snapshot; later rounds/addresses must not extend its deadline.
    pub fn application_path_policy(&self) -> ApplicationPathPolicySnapshot {
        let initial_pto = self.application_initial_pto();
        ApplicationPathPolicySnapshot {
            enabled: self.config.application_path_policy,
            generation: self.application_path_policy.generation,
            revision: self.application_path_policy.revision,
            requires_new_generation: self.application_path_policy.requires_new_generation,
            network_change_in_progress: self.application_network_change_pending(),
            network_change_count: self.application_path_policy.network_change_count,
            application_tx_bytes: self.application_path_policy.application_tx_bytes,
            application_rx_bytes: self.application_path_policy.application_rx_bytes,
            paths: self.paths.iter().map(|(&id, path)| {
                let data = &path.data;
                ApplicationPathObservation {
                    token: ApplicationPathToken {
                        connection: self.application_path_policy.connection,
                        generation: self.application_path_policy.generation,
                        revision: self.application_path_policy.revision,
                        path_id: id,
                        path_generation: data.generation(),
                    },
                    network_path: data.network_path,
                    validated: self.state.is_established() && data.validated
                        && !self.application_network_change_pending()
                        && !self.abandoned_paths.contains(&id)
                        && !self.application_path_policy.pending_validation.contains(&id),
                    data_allowed: self.application_data_allowed_on_path(id),
                    retirement_allowed: self.application_path_retirement_allowed(id),
                    validation_in_progress: data.is_validating_path(),
                    previous_validation_in_progress: path.prev.as_ref().is_some_and(|(_, prev)| prev.is_validating_path()),
                    validation_deadline: self.timers.get(Timer::PerPath(id, PathTimer::PathValidationFailed)),
                    challenge_retry_deadline: self.timers.get(Timer::PerPath(id, PathTimer::PathChallengeLost)),
                    application_tx_bytes: self.application_path_policy.path_payload_bytes.get(&id).map_or(0, |bytes| bytes.0),
                    application_rx_bytes: self.application_path_policy.path_payload_bytes.get(&id).map_or(0, |bytes| bytes.1),
                    pto: self.pto(super::SpaceKind::Data, id),
                }
            }).collect(),
            nat_round: self.n0_nat_traversal.observation(self.config.initial_rtt),
            nat_retry_deadline: self.timers.get(Timer::Conn(ConnTimer::NatTraversalProbeRetry)),
            initial_rtt: self.config.initial_rtt,
            initial_pto,
            best_validated_pto: self.application_best_pto(),
        }
    }

    /// Select or revoke application sending under an explicitly enabled policy.
    ///
    /// This must share the send-path serialization boundary. Noq supplies that
    /// boundary with its connection mutex and invalidates retained I/O buffers.
    /// STREAM identity and retransmission queues survive policy changes.
    pub fn update_application_path_policy(
        &mut self,
        now: Instant,
        update: ApplicationPathUpdate,
    ) -> Result<(), ApplicationPathPolicyError> {
        use ApplicationPathPolicyError as Error;
        if !self.config.application_path_policy { return Err(Error::Disabled); }
        if self.state.is_closed() { return Err(Error::Closed); }
        let validation_reference_pto = self.application_validation_reference_pto();
        let policy = &self.application_path_policy;
        let (generation, admitted) = match update {
            ApplicationPathUpdate::BeginGeneration(generation) => {
                if generation <= policy.generation { return Err(Error::StaleGeneration); }
                (generation, None)
            }
            ApplicationPathUpdate::Block(generation) => {
                if generation != policy.generation { return Err(Error::StaleGeneration); }
                (generation, None)
            }
            ApplicationPathUpdate::Allow(token) => {
                if policy.requires_new_generation || self.application_network_change_pending() {
                    return Err(Error::NetworkGenerationRequired);
                }
                self.check_application_path_token(token)?;
                let path = &self.paths[&token.path_id];
                if !self.state.is_established() || !path.data.validated
                    || policy.pending_validation.contains(&token.path_id)
                    || self.abandoned_paths.contains(&token.path_id)
                    || !self.remote_cids.contains_key(&token.path_id) {
                    return Err(Error::UnvalidatedPath);
                }
                (policy.generation, Some((token.path_id, token.path_generation)))
            }
            ApplicationPathUpdate::RetireRedundantPath(token) => {
                self.check_application_path_token(token)?;
                (policy.generation, policy.admitted)
            }
        };
        let Some(revision) = policy.revision.checked_add(1) else {
            self.application_path_policy.admitted = None;
            self.application_path_policy.requires_new_generation = true;
            return Err(Error::RevisionExhausted);
        };
        if let ApplicationPathUpdate::RetireRedundantPath(token) = update {
            // close_path performs the single revision change and retains the
            // grant on the independent surviving path.
            return self.retire_redundant_application_path(now, token.path_id);
        }
        if matches!(update, ApplicationPathUpdate::BeginGeneration(_)) {
            self.application_path_policy.requires_new_generation = false;
        }
        self.application_path_policy.generation = generation;
        self.application_path_policy.revision = revision;
        self.application_path_policy.admitted = admitted;
        if matches!(update, ApplicationPathUpdate::BeginGeneration(_))
            && !self.application_network_change_pending() {
            self.start_application_path_validation(now, validation_reference_pto);
        }
        Ok(())
    }

    /// Whether explicit application admission was enabled before connection creation.
    pub fn application_path_policy_enabled(&self) -> bool {
        self.config.application_path_policy
    }

    /// Whether the selected path currently admits application frames.
    pub fn application_data_allowed(&self) -> bool {
        !self.config.application_path_policy || self.application_path_policy.admitted
            .is_some_and(|(id, _)| self.application_data_allowed_on_path(id))
    }

    /// Current stamp for an encrypted transmit retained outside Noq-proto.
    ///
    /// A changed stamp requires discarding those bytes. Reliable frames remain
    /// tracked as in flight and recover through the existing loss machinery;
    /// DATAGRAM retains its existing best-effort semantics.
    pub fn application_transmit_stamp(&self) -> Option<ApplicationTransmitStamp> {
        self.config.application_path_policy.then(|| ApplicationTransmitStamp {
            connection: self.application_path_policy.connection,
            generation: self.application_path_policy.generation,
            revision: self.application_path_policy.revision,
            path_generation_counter: self.path_generation_counter,
            admitted: self.application_path_policy.admitted
                .filter(|(id, _)| self.application_data_allowed_on_path(*id)),
        })
    }

    pub(super) fn invalidate_application_path(&mut self, id: PathId) {
        if !self.config.application_path_policy { return; }
        self.application_path_policy.pending_validation.insert(id);
        if self.application_path_policy.admitted.is_some_and(|(path, _)| path == id) {
            self.application_path_policy.admitted = None;
        }
        // Saturation is fail-closed: subsequent explicit updates reject exhaustion.
        self.application_path_policy.revision = self.application_path_policy.revision.saturating_add(1);
    }

    /// Revoke application admission before a confirmed network change reuses I/O.
    ///
    /// The serialized endpoint pairs this with `handle_network_change` once the
    /// network is usable. New generations may start while waiting, but cannot
    /// admit data until that completion and fresh on-path validation.
    pub fn begin_application_network_change(&mut self) {
        if !self.config.application_path_policy || self.state.is_drained() { return; }
        let reference_pto = self.revoke_application_network();
        let pending = &mut self.application_path_policy.pending_network_change_pto;
        *pending = Some(pending.map_or(reference_pto, |previous| previous.max(reference_pto)));
    }

    pub(super) fn application_network_change_pending(&self) -> bool {
        self.application_path_policy.pending_network_change_pto.is_some()
    }

    pub(super) fn complete_application_network_change(&mut self, now: Instant) {
        if let Some(reference_pto) = self.application_path_policy.pending_network_change_pto.take() {
            self.application_path_policy.admitted = None;
            self.application_path_policy.revision = self.application_path_policy.revision.saturating_add(1);
            self.application_path_policy.requires_new_generation = true;
            let reference_pto = reference_pto.max(self.application_validation_reference_pto());
            self.start_application_path_validation(now, reference_pto);
        } else {
            self.invalidate_application_network(now);
        }
    }

    pub(super) fn invalidate_application_network(&mut self, now: Instant) {
        if !self.config.application_path_policy { return; }
        let reference_pto = self.revoke_application_network();
        self.start_application_path_validation(now, reference_pto);
    }

    fn revoke_application_network(&mut self) -> Duration {
        let validation_reference_pto = self.application_validation_reference_pto();
        self.application_path_policy.admitted = None;
        self.application_path_policy.revision = self.application_path_policy.revision.saturating_add(1);
        self.application_path_policy.requires_new_generation = true;
        self.application_path_policy.network_change_count = self.application_path_policy.network_change_count.saturating_add(1);
        validation_reference_pto
    }

    fn check_application_path_token(&self, token: ApplicationPathToken) -> Result<(), ApplicationPathPolicyError> {
        use ApplicationPathPolicyError as Error;
        let policy = &self.application_path_policy;
        if token.generation != policy.generation { return Err(Error::StaleGeneration); }
        if token.connection != policy.connection || token.revision != policy.revision {
            return Err(Error::StaleObservation);
        }
        let path = self.paths.get(&token.path_id).ok_or(Error::StaleObservation)?;
        if path.data.generation() != token.path_generation { return Err(Error::StaleObservation); }
        Ok(())
    }

    pub(super) fn application_data_allowed_on_path(&self, id: PathId) -> bool {
        if !self.config.application_path_policy { return true; }
        self.state.is_established() && !self.abandoned_paths.contains(&id)
            && !self.application_network_change_pending()
            && !self.application_path_policy.requires_new_generation
            && !self.application_path_policy.pending_validation.contains(&id)
            && self.remote_cids.contains_key(&id)
            && self.paths.get(&id).is_some_and(|path| {
                path.data.validated && self.application_path_policy.admitted
                    == Some((id, path.data.generation()))
            })
    }

    pub(super) fn record_application_payload_tx(&mut self, id: PathId, bytes: u64) {
        if !self.config.application_path_policy || bytes == 0 { return; }
        let policy = &mut self.application_path_policy;
        policy.application_tx_bytes = policy.application_tx_bytes.saturating_add(bytes);
        let path = policy.path_payload_bytes.entry(id).or_default();
        path.0 = path.0.saturating_add(bytes);
    }

    pub(super) fn record_application_payload_rx(&mut self, id: PathId, bytes: u64) {
        if !self.config.application_path_policy || bytes == 0 { return; }
        let policy = &mut self.application_path_policy;
        policy.application_rx_bytes = policy.application_rx_bytes.saturating_add(bytes);
        let path = policy.path_payload_bytes.entry(id).or_default();
        path.1 = path.1.saturating_add(bytes);
    }

    pub(super) fn application_initial_pto(&self) -> Duration {
        let rtt = self.config.initial_rtt;
        rtt + (4 * (rtt / 2)).max(super::TIMER_GRANULARITY)
            + self.ack_frequency.max_ack_delay_for_pto()
    }

    pub(super) fn application_best_pto(&self) -> Duration {
        self.paths.iter().filter(|(id, path)| path.data.validated
            && !self.abandoned_paths.contains(id))
            .map(|(&id, _)| self.pto(super::SpaceKind::Data, id))
            .min().unwrap_or_else(|| self.application_initial_pto())
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use super::*;
    use crate::{FrameStats, VarInt, frame, connection::{PacketBuilder, TransmitBuf, datagrams::DatagramState}};

    #[test]
    fn route_policy_task_serializer_blocks_payload_and_redacts_close() {
        let mut bytes = Vec::new();
        let mut transmit = TransmitBuf::new(&mut bytes, std::num::NonZeroUsize::MIN, 1200);
        transmit.start_new_datagram();
        let mut builder = PacketBuilder::policy_data_buf(&mut transmit, false);
        let mut stats = FrameStats::default();
        builder.write_frame(frame::Datagram { data: Bytes::from_static(b"private") }, &mut stats);
        builder.write_frame(frame::StreamMeta {
            id: crate::StreamId(0), offsets: 0..7, fin: false,
        }.encoder(true), &mut stats);
        assert_eq!(builder.buf.len(), 0);
        builder.write_frame(frame::Ping, &mut stats);
        let close = frame::Close::Application(crate::ApplicationClose {
            error_code: VarInt::from_u32(7), reason: Bytes::from_static(b"private close reason"),
        });
        builder.write_frame(close.encoder(256), &mut stats);
        drop(builder);
        let mut frames = frame::Iter::new(Bytes::copy_from_slice(transmit.as_mut_slice())).unwrap();
        assert!(matches!(frames.next().unwrap().unwrap(), frame::Frame::Ping));
        assert!(matches!(frames.next().unwrap().unwrap(),
            frame::Frame::Close(frame::Close::Application(close))
                if close.reason.is_empty() && close.error_code == VarInt::from_u32(7)));
        assert!(frames.next().is_none());
    }

    #[test]
    fn route_policy_task_datagram_queue_survives_block_and_grant() {
        let mut bytes = Vec::new();
        let mut transmit = TransmitBuf::new(&mut bytes, std::num::NonZeroUsize::MIN, 1200);
        transmit.start_new_datagram();
        let mut datagrams = DatagramState::default();
        datagrams.outgoing.push_back(frame::Datagram { data: Bytes::from_static(b"payload") });
        datagrams.outgoing_total = 7;
        let mut stats = FrameStats::default();
        let mut blocked = PacketBuilder::policy_data_buf(&mut transmit, false);
        assert!(!datagrams.write(&mut blocked, &mut stats));
        assert_eq!(datagrams.outgoing_total, 7);
        assert_eq!(datagrams.outgoing.len(), 1);
        drop(blocked);
        let mut allowed = PacketBuilder::policy_data_buf(&mut transmit, true);
        assert!(datagrams.write(&mut allowed, &mut stats));
        assert_eq!(datagrams.outgoing_total, 0);
        assert!(datagrams.outgoing.is_empty());
        drop(allowed);
        let frame = frame::Iter::new(Bytes::copy_from_slice(transmit.as_mut_slice()))
            .unwrap().next().unwrap().unwrap();
        assert!(matches!(frame, frame::Frame::Datagram(frame) if frame.data.as_ref() == b"payload"));
    }
}
