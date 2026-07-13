use std::num::NonZeroUsize;
use std::time::Instant;

use lru::LruCache;

use super::limits::SourceKey;

const ACCEPT_SOURCE_CACHE_CAPACITY: usize = 4_096;
const ACCEPTS_PER_SECOND_GLOBAL: f64 = 128.0;
const ACCEPT_BURST_GLOBAL: f64 = 256.0;
const ACCEPTS_PER_SECOND_PER_SOURCE: f64 = 16.0;
const ACCEPT_BURST_PER_SOURCE: f64 = 32.0;

const INBOUND_MESSAGES_PER_SECOND: f64 = 128.0;
const INBOUND_MESSAGE_BURST: f64 = 128.0;
const INBOUND_BYTES_PER_SECOND: f64 = (2 * 1024 * 1024) as f64;
const INBOUND_BYTE_BURST: f64 = (2 * 1024 * 1024) as f64;

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
        let started = Instant::now();
        let mut limiter = AcceptRateLimiter::new_at(started);
        let source = SourceKey::Ipv4([192, 0, 2, 1]);
        for _ in 0..ACCEPT_BURST_PER_SOURCE as usize {
            assert!(limiter.try_admit_at(source, started));
        }
        assert!(!limiter.try_admit_at(source, started));
        assert!(limiter.try_admit_at(source, started + Duration::from_secs(1)));
    }

    #[test]
    fn proxy_still_consumes_global_accept_budget() {
        let started = Instant::now();
        let mut limiter = AcceptRateLimiter::new_at(started);
        for _ in 0..ACCEPT_BURST_GLOBAL as usize {
            assert!(limiter.try_admit_at(SourceKey::ExternallyLimitedProxy, started));
        }
        assert!(!limiter.try_admit_at(SourceKey::ExternallyLimitedProxy, started));
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
