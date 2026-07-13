use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use tokio::sync::OwnedSemaphorePermit;

pub(super) struct ApplicationHandshakePermit {
    _global: OwnedSemaphorePermit,
    _peer: PeerHandshakePermit,
}

impl ApplicationHandshakePermit {
    pub(super) fn new(global: OwnedSemaphorePermit, peer: PeerHandshakePermit) -> Self {
        Self {
            _global: global,
            _peer: peer,
        }
    }
}

#[derive(Clone)]
pub(super) struct PeerHandshakeLimiter {
    inner: Arc<PeerHandshakeLimiterInner>,
}

struct PeerHandshakeLimiterInner {
    active: Mutex<HashMap<String, usize>>,
    max_per_peer: usize,
    max_peers: usize,
}

impl PeerHandshakeLimiter {
    pub(super) fn new(max_per_peer: usize, max_peers: usize) -> Self {
        Self {
            inner: Arc::new(PeerHandshakeLimiterInner {
                active: Mutex::new(HashMap::new()),
                max_per_peer: max_per_peer.max(1),
                max_peers: max_peers.max(1),
            }),
        }
    }

    pub(super) fn try_acquire(&self, peer: &str) -> io::Result<PeerHandshakePermit> {
        if peer.is_empty() || peer.len() > 128 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "invalid remote endpoint identity",
            ));
        }
        let mut active = self
            .inner
            .active
            .lock()
            .map_err(|_| io::Error::other("peer handshake limiter is locked"))?;
        if let Some(count) = active.get_mut(peer) {
            if *count >= self.inner.max_per_peer {
                return Err(limit_reached());
            }
            *count += 1;
        } else {
            if active.len() >= self.inner.max_peers {
                return Err(limit_reached());
            }
            active.insert(peer.to_string(), 1);
        }
        Ok(PeerHandshakePermit {
            limiter: self.clone(),
            peer: peer.to_string(),
        })
    }

    #[cfg(test)]
    fn active_peers(&self) -> usize {
        self.inner.active.lock().unwrap().len()
    }
}

pub(super) struct PeerHandshakePermit {
    limiter: PeerHandshakeLimiter,
    peer: String,
}

impl Drop for PeerHandshakePermit {
    fn drop(&mut self) {
        let Ok(mut active) = self.limiter.inner.active.lock() else {
            return;
        };
        let Some(count) = active.get_mut(&self.peer) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            active.remove(&self.peer);
        }
    }
}

fn limit_reached() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "remote handshake admission limit reached",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_each_endpoint_and_releases_capacity() {
        let limiter = PeerHandshakeLimiter::new(2, 8);
        let first = limiter.try_acquire("peer-a").unwrap();
        let second = limiter.try_acquire("peer-a").unwrap();
        assert_eq!(
            limiter.try_acquire("peer-a").err().unwrap().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(limiter.try_acquire("peer-b").is_ok());
        drop(first);
        assert!(limiter.try_acquire("peer-a").is_ok());
        drop(second);
    }

    #[test]
    fn distinct_endpoint_map_is_bounded_and_pruned() {
        let limiter = PeerHandshakeLimiter::new(1, 2);
        let first = limiter.try_acquire("peer-a").unwrap();
        let _second = limiter.try_acquire("peer-b").unwrap();
        assert_eq!(limiter.active_peers(), 2);
        assert_eq!(
            limiter.try_acquire("peer-c").err().unwrap().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(first);
        assert_eq!(limiter.active_peers(), 1);
        assert!(limiter.try_acquire("peer-c").is_ok());
    }
}
