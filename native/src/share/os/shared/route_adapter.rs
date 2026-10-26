//! The only app module using Iroh 1.3.0's unstable selector API.
use super::route_diagnostics::milliseconds;
use super::route_policy::{RoundFacts, RouteCandidate};
use super::route_state::RouteClass;
use iroh::endpoint::transports::{Addr, PathSelection, PathSelectionContext, PathSelector};
use iroh::endpoint::{
    ApplicationPathPolicySnapshot, ApplicationPathToken, Connection, LocalTransportAddr,
};
use iroh::TransportAddr;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub(super) struct RouteSelector {
    evidence: Mutex<InterfaceCache>,
}

#[derive(Debug, Default)]
struct InterfaceCache(HashMap<(Option<IpAddr>, SocketAddr), (Instant, bool)>);

impl InterfaceCache {
    fn lan(&mut self, local: Option<IpAddr>, remote: SocketAddr) -> bool {
        let key = (local, remote);
        if let Some((at, result)) = self.0.get(&key) {
            if at.elapsed() < Duration::from_secs(1) {
                return *result;
            }
        }
        if self.0.len() >= 128 {
            self.0.clear();
        }
        let result = super::route_interfaces::physical_on_link(local, remote);
        self.0.insert(key, (Instant::now(), result));
        result
    }
}

impl PathSelector for RouteSelector {
    fn select(&self, context: &PathSelectionContext<'_>) -> PathSelection {
        let mut selection = PathSelection::none();
        let Ok(mut cache) = self.evidence.lock() else {
            return selection;
        };
        let best = context
            .paths()
            .filter_map(|path| {
                let stats = path.stats()?;
                let local = match path.network_path().local() {
                    LocalTransportAddr::Ip(local) => local,
                    _ => None,
                };
                let rank = match path.network_path().remote() {
                    Addr::Ip(remote) if cache.lan(local, remote) => 0,
                    Addr::Relay(..) => 2,
                    Addr::Ip(_) | Addr::Custom(_) => 1,
                };
                Some((path, (rank, stats.rtt)))
            })
            .min_by_key(|(_, rank)| *rank);
        if let Some((path, _)) = best {
            selection.set(&path);
        }
        // This steers transport preference; only the serialized QUIC gate grants payload.
        selection
    }
}

#[derive(Default)]
pub(super) struct RouteAdapter {
    evidence: InterfaceCache,
}

pub(super) struct Observation {
    pub(super) paths: Vec<RouteCandidate>,
    pub(super) tokens: HashMap<String, ApplicationPathToken>,
    pub(super) round: Option<RoundFacts>,
}

impl RouteAdapter {
    pub(super) fn observe(
        &mut self,
        connection: &Connection,
        snapshot: &ApplicationPathPolicySnapshot,
        origin: Instant,
        hints: &[SocketAddr],
    ) -> Observation {
        let paths = connection.paths();
        let mut candidates = Vec::new();
        let mut tokens = HashMap::new();
        for observed in snapshot.paths.iter().take(64) {
            let Some(path) = paths
                .iter()
                .find(|path| path.id() == observed.token.path_id())
            else {
                continue;
            };
            let local = match path.local_addr() {
                LocalTransportAddr::Ip(local) => *local,
                _ => None,
            };
            let class = match path.remote_addr() {
                TransportAddr::Ip(remote) if self.evidence.lan(local, *remote) => RouteClass::Lan,
                TransportAddr::Relay(_) => RouteClass::Relay,
                _ => RouteClass::Direct,
            };
            let key = format!("path/{}", path.id());
            let snapshot_key = match path.remote_addr() {
                TransportAddr::Ip(remote) => format!("candidate/{remote}"),
                _ => key.clone(),
            };
            candidates.push(RouteCandidate {
                key: key.clone(),
                snapshot_key,
                class,
                validated: observed.validated,
                validation_pending: observed.validation_in_progress
                    || observed.previous_validation_in_progress,
                rtt: connection.rtt(path.id()),
            });
            tokens.insert(key, observed.token);
        }
        let mut hints = hints.iter().copied().take(32).collect::<Vec<_>>();
        let round = snapshot.nat_round.as_ref().map(|round| {
            hints.extend(
                round
                    .candidates
                    .iter()
                    .take(32)
                    .map(|candidate| candidate.remote),
            );
            RoundFacts {
                round: round.round.to_string(),
                candidates: round
                    .candidates
                    .iter()
                    .take(64)
                    .map(|candidate| format!("candidate/{}", candidate.remote))
                    .collect(),
                exhausted: round.retry_work_exhausted,
                response_deadline_ms: round
                    .last_response_deadline
                    .map(|deadline| milliseconds(deadline.saturating_duration_since(origin))),
                pending_path_opens: round.pending_path_opens,
                retry_intervals: round.retry_intervals.to_vec(),
                max_probe_attempts: round.max_probe_attempts,
            }
        });
        hints.sort();
        hints.dedup();
        for remote in hints {
            candidates.push(RouteCandidate {
                key: format!("candidate/{remote}"),
                snapshot_key: format!("candidate/{remote}"),
                class: if self.evidence.lan(None, remote) {
                    RouteClass::Lan
                } else {
                    RouteClass::Direct
                },
                validated: false,
                validation_pending: false,
                rtt: None,
            });
        }
        Observation {
            paths: candidates,
            tokens,
            round,
        }
    }
}
