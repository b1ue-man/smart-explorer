use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
};

use super::connection_source::ConnectionSource;

/// Admission controller for accepted TCP connections.
///
/// A permit is acquired immediately after `accept` and retained until every
/// clone held by the HTTP handshake and relay connection actor has gone away.
#[derive(Clone, Debug)]
pub(super) struct ConnectionLimiter {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    max_total: Option<usize>,
    max_per_source: Option<usize>,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    total: usize,
    by_source: HashMap<ConnectionSource, usize>,
}

impl ConnectionLimiter {
    pub(super) fn new(max_total: Option<usize>, max_per_source: Option<usize>) -> Option<Self> {
        if max_total.is_none() && max_per_source.is_none() {
            return None;
        }
        Some(Self {
            inner: Arc::new(Inner {
                max_total,
                max_per_source,
                state: Mutex::new(State::default()),
            }),
        })
    }

    pub(super) fn try_acquire(&self, ip: IpAddr) -> Option<Arc<ConnectionPermit>> {
        let source = ConnectionSource::from_ip(ip);
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let source_count = state.by_source.get(&source).copied().unwrap_or(0);
        if self
            .inner
            .max_total
            .is_some_and(|maximum| state.total >= maximum)
            || self
                .inner
                .max_per_source
                .is_some_and(|maximum| source_count >= maximum)
        {
            return None;
        }
        let next_total = state.total.checked_add(1)?;
        let next_source_count = source_count.checked_add(1)?;
        state.total = next_total;
        state.by_source.insert(source, next_source_count);
        drop(state);
        Some(Arc::new(ConnectionPermit {
            inner: self.inner.clone(),
            source,
        }))
    }

    #[cfg(test)]
    pub(super) fn active_connections(&self) -> usize {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .total
    }
}

/// RAII ownership of one admitted TCP connection.
#[derive(Debug)]
pub(super) struct ConnectionPermit {
    inner: Arc<Inner>,
    source: ConnectionSource,
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        debug_assert!(state.total > 0, "connection permit total underflow");
        state.total = state.total.saturating_sub(1);
        let Some(source_count) = state.by_source.get_mut(&self.source) else {
            debug_assert!(false, "connection permit source missing");
            return;
        };
        debug_assert!(*source_count > 0, "connection permit source underflow");
        *source_count = source_count.saturating_sub(1);
        if *source_count == 0 {
            state.by_source.remove(&self.source);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    #[test]
    fn unset_limits_preserve_unlimited_upstream_behavior() {
        assert!(ConnectionLimiter::new(None, None).is_none());
    }

    #[test]
    fn total_limit_counts_different_sources() {
        let limiter = ConnectionLimiter::new(Some(2), None).unwrap();
        let first = limiter
            .try_acquire(Ipv4Addr::new(192, 0, 2, 1).into())
            .unwrap();
        let _second = limiter
            .try_acquire(Ipv4Addr::new(192, 0, 2, 2).into())
            .unwrap();
        assert!(
            limiter
                .try_acquire(Ipv4Addr::new(192, 0, 2, 3).into())
                .is_none()
        );

        drop(first);
        assert!(
            limiter
                .try_acquire(Ipv4Addr::new(192, 0, 2, 3).into())
                .is_some()
        );
    }

    #[test]
    fn per_source_limit_is_independent_between_ipv4_addresses() {
        let limiter = ConnectionLimiter::new(Some(4), Some(1)).unwrap();
        let _first = limiter
            .try_acquire(Ipv4Addr::new(198, 51, 100, 1).into())
            .unwrap();
        assert!(
            limiter
                .try_acquire(Ipv4Addr::new(198, 51, 100, 1).into())
                .is_none()
        );
        assert!(
            limiter
                .try_acquire(Ipv4Addr::new(198, 51, 100, 2).into())
                .is_some()
        );
    }

    #[test]
    fn ipv6_sources_are_normalized_to_64_bit_prefixes() {
        let first: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let same_prefix: IpAddr = "2001:db8:1:2:ffff::2".parse().unwrap();
        let other_prefix: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(
            ConnectionSource::from_ip(first),
            ConnectionSource::from_ip(same_prefix)
        );
        assert_ne!(
            ConnectionSource::from_ip(first),
            ConnectionSource::from_ip(other_prefix)
        );

        let limiter = ConnectionLimiter::new(None, Some(1)).unwrap();
        let _first = limiter.try_acquire(first).unwrap();
        assert!(limiter.try_acquire(same_prefix).is_none());
        assert!(limiter.try_acquire(other_prefix).is_some());

        let mapped = Ipv4Addr::new(203, 0, 113, 9).to_ipv6_mapped();
        assert_eq!(
            ConnectionSource::from_ip(mapped.into()),
            ConnectionSource::from_ip(Ipv4Addr::new(203, 0, 113, 9).into())
        );
    }

    #[test]
    fn capacity_is_reused_only_after_last_permit_clone_drops() {
        let limiter = ConnectionLimiter::new(Some(1), Some(1)).unwrap();
        let permit = limiter.try_acquire(Ipv4Addr::LOCALHOST.into()).unwrap();
        let retained_by_connection_actor = permit.clone();
        drop(permit);
        assert!(limiter.try_acquire(Ipv4Addr::LOCALHOST.into()).is_none());

        drop(retained_by_connection_actor);
        assert!(limiter.try_acquire(Ipv4Addr::LOCALHOST.into()).is_some());
    }
}
