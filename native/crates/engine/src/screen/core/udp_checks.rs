//! Connectivity checks of the direct UDP pair (`video_transport.rs`), as
//! ICE (RFC 8445 7.2) and its consent freshness (RFC 7675) do them: both
//! sides send sealed checks (`udp_wire.rs`) to every candidate of the
//! other; a check is answered to the address it came from, and an address
//! a check came from that was no candidate is checked as well (a
//! peer-reflexive one behind a NAT). A pair is valid once the answer to a
//! check comes from the very address the check went to (a replayed answer
//! from elsewhere validates nothing); the first valid pair is taken. Then a
//! check every `KEEPALIVE_US` keeps the NAT mappings open; without any
//! sealed packet from the pair for `CONSENT_US` it is dropped and the
//! checks start over. Times are µs of the caller; no socket here.
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;

use super::udp_wire::{Envelope, UdpType};
use super::MAX_CANDIDATES;

/// Checks to every candidate this often while no pair is valid, for the
/// first `FAST_FOR_US`; then every `CHECK_SLOW_US`.
pub const CHECK_FAST_US: u64 = 100_000;
pub const FAST_FOR_US: u64 = 5_000_000;
pub const CHECK_SLOW_US: u64 = 1_000_000;
/// A check on the valid pair this often (NAT mappings, consent).
pub const KEEPALIVE_US: u64 = 500_000;
/// The valid pair is dropped after this long without a sealed packet.
pub const CONSENT_US: u64 = 3_000_000;
/// Checks awaiting their answer at most, and for how long.
const PENDING: usize = 64;
const PENDING_US: u64 = 2_000_000;
/// Remote addresses at most: the candidates and the peer-reflexive ones.
pub const MAX_REMOTE: usize = 2 * MAX_CANDIDATES;

/// What an incoming packet asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    /// A picture packet for the assembler.
    Picture(Vec<u8>),
    /// The answer to a check, to send to `to`.
    Reply {
        packet: Vec<u8>,
        to: SocketAddr,
    },
    Nothing,
}

pub struct Checks {
    envelope: Arc<Envelope>,
    remote: Vec<SocketAddr>,
    pending: VecDeque<(u64, SocketAddr, u64)>,
    selected: Option<SocketAddr>,
    published: Option<SocketAddr>,
    heard_us: u64,
    started_us: u64,
    next_us: u64,
    next_transaction: u64,
}

impl Checks {
    /// `seed`: the first transaction id (random per side).
    pub fn new(envelope: Arc<Envelope>, remote: Vec<SocketAddr>, seed: u64, now_us: u64) -> Self {
        let mut remote = remote;
        remote.truncate(MAX_REMOTE);
        Self {
            envelope,
            remote,
            pending: VecDeque::new(),
            selected: None,
            published: None,
            heard_us: now_us,
            started_us: now_us,
            next_us: now_us,
            next_transaction: seed,
        }
    }

    pub fn envelope(&self) -> &Arc<Envelope> {
        &self.envelope
    }

    pub fn selected(&self) -> Option<SocketAddr> {
        self.selected
    }

    /// When `tick` has checks to send next.
    pub fn due_us(&self) -> u64 {
        match self.selected {
            Some(_) => self.next_us.min(self.heard_us.saturating_add(CONSENT_US)),
            None => self.next_us,
        }
    }

    /// The valid pair changed since the last call: the new one, or `None`
    /// when its consent ran out.
    pub fn take_change(&mut self) -> Option<Option<SocketAddr>> {
        (self.selected != self.published).then(|| {
            self.published = self.selected;
            self.selected
        })
    }

    /// The checks due at `now_us`, each with its destination.
    pub fn tick(&mut self, now_us: u64) -> Vec<(Vec<u8>, SocketAddr)> {
        if self.selected.is_some() && now_us.saturating_sub(self.heard_us) >= CONSENT_US {
            self.selected = None;
            self.started_us = now_us;
            self.next_us = now_us;
        }
        if now_us < self.next_us {
            return Vec::new();
        }
        let interval = if self.selected.is_some() {
            KEEPALIVE_US
        } else if now_us.saturating_sub(self.started_us) < FAST_FOR_US {
            CHECK_FAST_US
        } else {
            CHECK_SLOW_US
        };
        self.next_us = now_us.saturating_add(interval);
        self.pending
            .retain(|(_, _, sent)| now_us.saturating_sub(*sent) < PENDING_US);
        let targets = match self.selected {
            Some(pair) => vec![pair],
            None => self.remote.clone(),
        };
        let mut out = Vec::with_capacity(targets.len());
        for to in targets {
            let transaction = self.next_transaction;
            self.next_transaction = self.next_transaction.wrapping_add(1);
            let Ok(packet) = self
                .envelope
                .seal(UdpType::Check, &transaction.to_be_bytes())
            else {
                continue;
            };
            if self.pending.len() >= PENDING {
                self.pending.pop_front();
            }
            self.pending.push_back((transaction, to, now_us));
            out.push((packet, to));
        }
        out
    }

    /// A packet that came from `from` at `now_us`.
    pub fn on_packet(&mut self, packet: &[u8], from: SocketAddr, now_us: u64) -> Incoming {
        let Some((kind, inner)) = self.envelope.open(packet) else {
            return Incoming::Nothing;
        };
        if self.selected == Some(from) {
            self.heard_us = now_us;
        }
        match kind {
            UdpType::Picture => Incoming::Picture(inner),
            UdpType::Check if inner.len() == 8 => {
                if !self.remote.contains(&from) && self.remote.len() < MAX_REMOTE {
                    self.remote.push(from);
                }
                match self.envelope.seal(UdpType::Answer, &inner) {
                    Ok(packet) => Incoming::Reply { packet, to: from },
                    Err(_) => Incoming::Nothing,
                }
            }
            UdpType::Answer if inner.len() == 8 => {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&inner);
                let transaction = u64::from_be_bytes(bytes);
                let found = self
                    .pending
                    .iter()
                    .position(|(id, to, _)| *id == transaction && *to == from);
                if let Some(index) = found {
                    self.pending.remove(index);
                    if self.selected.is_none() {
                        self.selected = Some(from);
                        self.heard_us = now_us;
                        self.next_us = now_us.saturating_add(KEEPALIVE_US);
                    }
                }
                Incoming::Nothing
            }
            _ => Incoming::Nothing,
        }
    }
}

#[cfg(test)]
#[path = "udp_checks_tests.rs"]
mod tests;
