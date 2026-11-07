//! New key bindings per source (R-SERVER-02): a burst, then a slow refill.

use std::time::{Duration, Instant};

use super::{NewBindingRateLimiter, SourceKey};

#[test]
fn new_binding_tokens_are_per_source_and_refill_slowly() {
    let started = Instant::now();
    let source = SourceKey::Ipv4([192, 0, 2, 1]);
    let mut limiter = NewBindingRateLimiter::default();
    for index in 0..512 {
        assert!(limiter.try_take_at(source, started), "binding {index}");
    }
    assert!(!limiter.try_take_at(source, started), "the burst is 512");

    // One token every ten seconds afterwards.
    let later = started + Duration::from_secs(10);
    assert!(limiter.try_take_at(source, later));
    assert!(!limiter.try_take_at(source, later));
    assert!(!limiter.try_take_at(source, started + Duration::from_secs(19)));
    assert!(limiter.try_take_at(source, started + Duration::from_secs(30)));

    // Another source has its own full bucket.
    let other = SourceKey::Ipv4([192, 0, 2, 2]);
    assert!(limiter.try_take_at(other, started));
    // A trusted proxy limits its original clients itself: no bucket here.
    for _ in 0..2_000 {
        assert!(limiter.try_take_at(SourceKey::ExternallyLimitedProxy, started));
    }
}
