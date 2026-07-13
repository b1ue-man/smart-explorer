//! Token-bucket admission control for newly accepted TCP sockets.

use std::{
    net::IpAddr,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
    time::Instant,
};

use lru::LruCache;

use super::connection_source::ConnectionSource;

/// Bounds memory even when an attacker rotates through many source prefixes.
const SOURCE_BUCKET_CAPACITY: usize = 4096;

#[derive(Clone, Debug)]
pub(super) struct AcceptRateLimiter(Arc<Mutex<State>>);

#[derive(Debug)]
struct State {
    global: Option<TokenBucket>,
    sources: Option<SourceBuckets>,
}

#[derive(Debug)]
struct SourceBuckets {
    spec: RateSpec,
    buckets: LruCache<ConnectionSource, TokenBucket>,
}

#[derive(Clone, Copy, Debug)]
struct RateSpec {
    tokens_per_second: f64,
    burst: f64,
}

#[derive(Debug)]
struct TokenBucket {
    spec: RateSpec,
    tokens: f64,
    updated_at: Instant,
}

impl AcceptRateLimiter {
    pub(super) fn new(
        global_rate: Option<f64>,
        global_burst: Option<usize>,
        source_rate: Option<f64>,
        source_burst: Option<usize>,
    ) -> Result<Option<Self>, String> {
        Self::new_at(
            global_rate,
            global_burst,
            source_rate,
            source_burst,
            Instant::now(),
        )
    }

    fn new_at(
        global_rate: Option<f64>,
        global_burst: Option<usize>,
        source_rate: Option<f64>,
        source_burst: Option<usize>,
        now: Instant,
    ) -> Result<Option<Self>, String> {
        let global = RateSpec::from_options("global", global_rate, global_burst)?;
        let source = RateSpec::from_options("per-source", source_rate, source_burst)?;
        if global.is_none() && source.is_none() {
            return Ok(None);
        }
        let capacity = NonZeroUsize::new(SOURCE_BUCKET_CAPACITY)
            .expect("source bucket capacity is a non-zero constant");
        Ok(Some(Self(Arc::new(Mutex::new(State {
            global: global.map(|spec| TokenBucket::new(spec, now)),
            sources: source.map(|spec| SourceBuckets {
                spec,
                buckets: LruCache::new(capacity),
            }),
        })))))
    }

    pub(super) fn try_accept(&self, ip: IpAddr) -> bool {
        self.try_accept_at(ip, Instant::now())
    }

    fn try_accept_at(&self, ip: IpAddr, now: Instant) -> bool {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(global) = state.global.as_mut() {
            global.refill(now);
            if !global.has_token() {
                return false;
            }
        }

        if let Some(sources) = state.sources.as_mut() {
            let spec = sources.spec;
            let bucket = sources
                .buckets
                .get_or_insert_mut(ConnectionSource::from_ip(ip), || {
                    TokenBucket::new(spec, now)
                });
            bucket.refill(now);
            if !bucket.has_token() {
                return false;
            }
            bucket.consume();
        }

        if let Some(global) = state.global.as_mut() {
            global.consume();
        }
        true
    }

    #[cfg(test)]
    fn source_cache_len(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .sources
            .as_ref()
            .map_or(0, |sources| sources.buckets.len())
    }
}

impl RateSpec {
    fn from_options(
        scope: &str,
        rate: Option<f64>,
        burst: Option<usize>,
    ) -> Result<Option<Self>, String> {
        let Some(tokens_per_second) = rate else {
            return Ok(None);
        };
        if !tokens_per_second.is_finite() || tokens_per_second <= 0.0 {
            return Err(format!(
                "{scope} accept rate must be a positive finite number"
            ));
        }
        let burst = burst.unwrap_or_else(|| tokens_per_second.ceil() as usize);
        if burst == 0 {
            return Err(format!("{scope} accept burst must be greater than zero"));
        }
        Ok(Some(Self {
            tokens_per_second,
            burst: burst as f64,
        }))
    }
}

impl TokenBucket {
    fn new(spec: RateSpec, now: Instant) -> Self {
        Self {
            spec,
            tokens: spec.burst,
            updated_at: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.updated_at);
        self.tokens = (self.tokens + elapsed.as_secs_f64() * self.spec.tokens_per_second)
            .min(self.spec.burst);
        self.updated_at = now;
    }

    fn has_token(&self) -> bool {
        self.tokens >= 1.0
    }

    fn consume(&mut self) {
        debug_assert!(self.has_token(), "token bucket underflow");
        self.tokens -= 1.0;
    }
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, time::Duration};

    use super::*;

    fn limiter_at(
        global_rate: Option<f64>,
        global_burst: Option<usize>,
        source_rate: Option<f64>,
        source_burst: Option<usize>,
        now: Instant,
    ) -> AcceptRateLimiter {
        AcceptRateLimiter::new_at(global_rate, global_burst, source_rate, source_burst, now)
            .expect("valid test rate")
            .expect("configured test limiter")
    }

    #[test]
    fn unconfigured_rates_preserve_unlimited_behavior() {
        assert!(
            AcceptRateLimiter::new(None, None, None, None)
                .unwrap()
                .is_none()
        );
        assert!(
            AcceptRateLimiter::new(None, Some(512), None, Some(64))
                .unwrap()
                .is_none(),
            "bursts without rates do not enable admission limiting"
        );
    }

    #[test]
    fn global_burst_denies_and_refills_deterministically() {
        let now = Instant::now();
        let limiter = limiter_at(Some(2.0), Some(2), None, None, now);
        let source = Ipv4Addr::new(192, 0, 2, 1).into();

        assert!(limiter.try_accept_at(source, now));
        assert!(limiter.try_accept_at(source, now));
        assert!(!limiter.try_accept_at(source, now));
        assert!(!limiter.try_accept_at(source, now + Duration::from_millis(499)));
        assert!(limiter.try_accept_at(source, now + Duration::from_millis(500)));
    }

    #[test]
    fn source_buckets_normalize_addresses_and_remain_independent() {
        let now = Instant::now();
        let limiter = limiter_at(None, None, Some(1.0), Some(1), now);
        let ipv4 = Ipv4Addr::new(198, 51, 100, 1);
        let other_ipv4 = Ipv4Addr::new(198, 51, 100, 2);
        assert!(limiter.try_accept_at(ipv4.into(), now));
        assert!(!limiter.try_accept_at(ipv4.to_ipv6_mapped().into(), now));
        assert!(limiter.try_accept_at(other_ipv4.into(), now));

        let ipv6: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let same_prefix: IpAddr = "2001:db8:1:2:ffff::2".parse().unwrap();
        let other_prefix: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert!(limiter.try_accept_at(ipv6, now));
        assert!(!limiter.try_accept_at(same_prefix, now));
        assert!(limiter.try_accept_at(other_prefix, now));
    }

    #[test]
    fn source_denial_does_not_spend_global_capacity() {
        let now = Instant::now();
        let limiter = limiter_at(Some(1.0), Some(2), Some(1.0), Some(1), now);
        let noisy_source = Ipv4Addr::new(192, 0, 2, 1).into();
        let other_source = Ipv4Addr::new(192, 0, 2, 2).into();
        let third_source = Ipv4Addr::new(192, 0, 2, 3).into();

        assert!(limiter.try_accept_at(noisy_source, now));
        assert!(!limiter.try_accept_at(noisy_source, now));
        assert!(
            limiter.try_accept_at(other_source, now),
            "per-source denial must not consume the remaining global token"
        );
        assert!(!limiter.try_accept_at(third_source, now));
        assert!(limiter.try_accept_at(noisy_source, now + Duration::from_secs(1)));
    }

    #[test]
    fn rotating_sources_cannot_grow_the_lru_cache() {
        let now = Instant::now();
        let limiter = limiter_at(None, None, Some(1.0), Some(1), now);
        for index in 0..=SOURCE_BUCKET_CAPACITY {
            assert!(limiter.try_accept_at(Ipv4Addr::from(index as u32).into(), now));
        }
        assert_eq!(limiter.source_cache_len(), SOURCE_BUCKET_CAPACITY);
        assert!(
            limiter.try_accept_at(Ipv4Addr::from(0_u32).into(), now),
            "the least-recent source was evicted and starts with one bounded burst"
        );
        assert_eq!(limiter.source_cache_len(), SOURCE_BUCKET_CAPACITY);
    }

    #[test]
    fn invalid_enabled_rates_are_rejected() {
        for rate in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(AcceptRateLimiter::new(Some(rate), Some(1), None, None).is_err());
        }
        assert!(AcceptRateLimiter::new(Some(1.0), Some(0), None, None).is_err());
        assert!(AcceptRateLimiter::new(None, None, Some(1.0), Some(0)).is_err());
    }
}
