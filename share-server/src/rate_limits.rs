use std::num::NonZeroUsize;
use std::time::Instant;

use lru::LruCache;

use super::limits::{NetworkKey, SourceKey};

const ACCEPT_SOURCE_CACHE_CAPACITY: usize = 4_096;
const ACCEPTS_PER_SECOND_GLOBAL: f64 = 128.0;
const ACCEPT_BURST_GLOBAL: f64 = 256.0;
const ACCEPTS_PER_SECOND_PER_SOURCE: f64 = 16.0;
const ACCEPT_BURST_PER_SOURCE: f64 = 32.0;

const INBOUND_MESSAGES_PER_SECOND: f64 = 128.0;
// Two full publish waves can follow immediately: initial state and usable
// relay routes. A legacy wave may also request each watched Direct relation.
pub(super) const INBOUND_MESSAGE_BURST: f64 = (2
    * (1 + super::limits::MAX_PUBLISHED_DIRECTS_PER_CLIENT
        + 2 * super::limits::MAX_WATCHES_PER_CLIENT
        + super::limits::MAX_ROOMS_PER_CLIENT))
    as f64;
const INBOUND_BYTES_PER_SECOND: f64 = (2 * 1024 * 1024) as f64;
const INBOUND_BYTE_BURST: f64 = INBOUND_MESSAGE_BURST * (16 * 1024) as f64;

pub(super) struct AcceptRateLimiter {
    global: TokenBucket,
    by_source: LruCache<SourceKey, TokenBucket>,
}

impl AcceptRateLimiter {
    pub(super) fn new() -> Self {
        Self::new_at(Instant::now())
    }

    fn new_at(now: Instant) -> Self {
        Self {
            global: TokenBucket::new(ACCEPTS_PER_SECOND_GLOBAL, ACCEPT_BURST_GLOBAL, now),
            by_source: LruCache::new(
                NonZeroUsize::new(ACCEPT_SOURCE_CACHE_CAPACITY)
                    .expect("accept source cache capacity is non-zero"),
            ),
        }
    }

    pub(super) fn try_admit(&mut self, source: SourceKey) -> bool {
        self.try_admit_at(source, Instant::now())
    }

    fn try_admit_at(&mut self, source: SourceKey, now: Instant) -> bool {
        self.global.refill(now);
        let source_allowed = if source.has_internal_source_limit() {
            let bucket = self.by_source.get_or_insert_mut(source, || {
                TokenBucket::new(ACCEPTS_PER_SECOND_PER_SOURCE, ACCEPT_BURST_PER_SOURCE, now)
            });
            bucket.refill(now);
            bucket.can_consume(1.0)
        } else {
            true
        };
        if !source_allowed || !self.global.can_consume(1.0) {
            return false;
        }
        self.global.consume(1.0);
        if source.has_internal_source_limit() {
            self.by_source
                .get_mut(&source)
                .expect("source bucket was inserted before admission")
                .consume(1.0);
        }
        true
    }
}

/// New device/lookup bindings per source (R-SERVER-02): keys cost nothing,
/// so the permanent binding tables are filled at most this fast per source.
const NEW_BINDINGS_PER_SECOND_PER_SOURCE: f64 = 0.1;
const NEW_BINDING_BURST_PER_SOURCE: f64 = 512.0;
/// New bindings of one IPv6 /56 end site, shared by its /64 sources (r-server
/// U4): a /56 holds 256 /64 sources, which must not multiply the budget.
const NEW_BINDINGS_PER_SECOND_PER_END_SITE: f64 = 0.1;
const NEW_BINDING_BURST_PER_END_SITE: f64 = 512.0;

/// Token buckets for new key bindings, by source and by IPv6 end site. The
/// externally limited proxy bucket has none (its proxy limits the original
/// clients); a binding takes one token of both buckets of its source.
pub(super) struct NewBindingRateLimiter {
    by_source: LruCache<SourceKey, TokenBucket>,
    by_end_site: LruCache<NetworkKey, TokenBucket>,
}

impl Default for NewBindingRateLimiter {
    fn default() -> Self {
        Self {
            by_source: LruCache::new(BINDING_SOURCE_CACHE_CAPACITY),
            by_end_site: LruCache::new(BINDING_SOURCE_CACHE_CAPACITY),
        }
    }
}

const BINDING_SOURCE_CACHE_CAPACITY: NonZeroUsize =
    match NonZeroUsize::new(ACCEPT_SOURCE_CACHE_CAPACITY) {
        Some(capacity) => capacity,
        None => NonZeroUsize::MIN,
    };

impl NewBindingRateLimiter {
    /// Takes one token of `source` for a new binding.
    pub(super) fn try_take(&mut self, source: SourceKey) -> bool {
        self.try_take_at(source, Instant::now())
    }

    pub(super) fn try_take_at(&mut self, source: SourceKey, now: Instant) -> bool {
        if !source.has_internal_source_limit() {
            return true;
        }
        let bucket = self.by_source.get_or_insert_mut(source, || {
            TokenBucket::new(
                NEW_BINDINGS_PER_SECOND_PER_SOURCE,
                NEW_BINDING_BURST_PER_SOURCE,
                now,
            )
        });
        bucket.refill(now);
        let mut end_site = None;
        if let Some(network) = source.network() {
            let end_site_bucket = self.by_end_site.get_or_insert_mut(network, || {
                TokenBucket::new(
                    NEW_BINDINGS_PER_SECOND_PER_END_SITE,
                    NEW_BINDING_BURST_PER_END_SITE,
                    now,
                )
            });
            end_site_bucket.refill(now);
            end_site = Some(end_site_bucket);
        }
        let end_site_allows = match &end_site {
            Some(end_site_bucket) => end_site_bucket.can_consume(1.0),
            None => true,
        };
        if !bucket.can_consume(1.0) || !end_site_allows {
            return false;
        }
        bucket.consume(1.0);
        if let Some(end_site_bucket) = end_site {
            end_site_bucket.consume(1.0);
        }
        true
    }
}

pub(super) struct InboundRateLimiter {
    messages: TokenBucket,
    bytes: TokenBucket,
}

impl InboundRateLimiter {
    pub(super) fn new() -> Self {
        Self::new_at(Instant::now())
    }

    fn new_at(now: Instant) -> Self {
        Self {
            messages: TokenBucket::new(INBOUND_MESSAGES_PER_SECOND, INBOUND_MESSAGE_BURST, now),
            bytes: TokenBucket::new(INBOUND_BYTES_PER_SECOND, INBOUND_BYTE_BURST, now),
        }
    }

    pub(super) fn try_consume(&mut self, bytes: usize) -> bool {
        self.try_consume_at(bytes, Instant::now())
    }

    /// Charges one fully assembled inbound message without charging its bytes again.
    ///
    /// WebSocket wire bytes are charged by the stream wrapper before Tungstenite parses frames.
    pub(super) fn try_consume_message(&mut self) -> bool {
        let now = Instant::now();
        self.messages.refill(now);
        if !self.messages.can_consume(1.0) {
            return false;
        }
        self.messages.consume(1.0);
        true
    }

    fn try_consume_at(&mut self, bytes: usize, now: Instant) -> bool {
        self.messages.refill(now);
        self.bytes.refill(now);
        let bytes = bytes.max(1) as f64;
        if !self.messages.can_consume(1.0) || !self.bytes.can_consume(bytes) {
            return false;
        }
        self.messages.consume(1.0);
        self.bytes.consume(bytes);
        true
    }
}

/// Per-connection token bucket for bytes read from the raw WebSocket stream.
pub(super) struct InboundByteRateLimiter {
    bytes: TokenBucket,
}

impl InboundByteRateLimiter {
    pub(super) fn new() -> Self {
        Self {
            bytes: TokenBucket::new(INBOUND_BYTES_PER_SECOND, INBOUND_BYTE_BURST, Instant::now()),
        }
    }

    pub(super) fn try_consume(&mut self, bytes: usize) -> bool {
        let now = Instant::now();
        self.bytes.refill(now);
        let bytes = bytes as f64;
        if !self.bytes.can_consume(bytes) {
            return false;
        }
        self.bytes.consume(bytes);
        true
    }

    #[cfg(test)]
    pub(super) fn fixed_burst(bytes: usize) -> Self {
        Self {
            bytes: TokenBucket::new(0.0, bytes as f64, Instant::now()),
        }
    }
}

struct TokenBucket {
    tokens: f64,
    refill_per_second: f64,
    capacity: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(refill_per_second: f64, capacity: f64, now: Instant) -> Self {
        Self {
            tokens: capacity,
            refill_per_second,
            capacity,
            last_refill: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last_refill);
        self.tokens =
            (self.tokens + elapsed.as_secs_f64() * self.refill_per_second).min(self.capacity);
        self.last_refill = now;
    }

    fn can_consume(&self, amount: f64) -> bool {
        self.tokens >= amount
    }

    fn consume(&mut self, amount: f64) {
        self.tokens -= amount;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn per_source_accept_burst_is_bounded_and_refills() {
        // Rows: a source with its own bucket, and the externally limited proxy,
        // which draws only on the global budget. Each row admits its burst,
        // refuses the next accept and admits again one second later.
        let started = Instant::now();
        let cases = [
            (
                "per source",
                SourceKey::Ipv4([192, 0, 2, 1]),
                ACCEPT_BURST_PER_SOURCE as usize,
            ),
            (
                "externally limited proxy",
                SourceKey::ExternallyLimitedProxy,
                ACCEPT_BURST_GLOBAL as usize,
            ),
        ];
        for (case, source, burst) in cases {
            let mut limiter = AcceptRateLimiter::new_at(started);
            for _ in 0..burst {
                assert!(limiter.try_admit_at(source, started), "{case}");
            }
            assert!(!limiter.try_admit_at(source, started), "{case}");
            assert!(
                limiter.try_admit_at(source, started + Duration::from_secs(1)),
                "{case}"
            );
        }
    }

    #[test]
    fn inbound_limits_messages_and_bytes_independently() {
        let started = Instant::now();
        let mut messages = InboundRateLimiter::new_at(started);
        for _ in 0..INBOUND_MESSAGE_BURST as usize {
            assert!(messages.try_consume_at(1, started));
        }
        assert!(!messages.try_consume_at(1, started));

        let mut bytes = InboundRateLimiter::new_at(started);
        assert!(bytes.try_consume_at(INBOUND_BYTE_BURST as usize, started));
        assert!(!bytes.try_consume_at(1, started));
        assert!(bytes.try_consume_at(1, started + Duration::from_secs(1)));
    }
}

#[cfg(test)]
#[path = "rate_limits_tests.rs"]
mod rate_limits_tests;

#[cfg(test)]
#[path = "unbelegt_binding_budget_task_tests.rs"]
mod unbelegt_binding_budget_task_tests;
