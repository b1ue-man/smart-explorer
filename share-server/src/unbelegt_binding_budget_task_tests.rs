//! Bindings of one IPv6 end site share one budget (r-server U4).

use std::time::{Duration, Instant};

use super::{NewBindingRateLimiter, NEW_BINDING_BURST_PER_END_SITE};
use crate::limits::SourceKey;

/// A /64 source inside the /56 `2001:db8:1:0::/56`; every index shares it.
fn source_in_site_one(index: u8) -> SourceKey {
    SourceKey::Ipv6Prefix64([0x20, 0x01, 0x0d, 0xb8, 0x00, 0x01, 0x00, index])
}

/// A /64 source of another /56, `2001:db8:2:0::/56`.
fn source_in_site_two() -> SourceKey {
    SourceKey::Ipv6Prefix64([0x20, 0x01, 0x0d, 0xb8, 0x00, 0x02, 0x00, 0x00])
}

#[test]
fn unbelegt_task_binding_budget_per_end_site() {
    let now = Instant::now();
    let mut limiter = NewBindingRateLimiter::default();
    // 256 /64 sources of one /56 try three bindings each: together they get
    // the burst of their end site once, not 256 bursts of their own.
    let mut admitted = 0_usize;
    for index in 0..=255_u8 {
        for _ in 0..3 {
            if limiter.try_take_at(source_in_site_one(index), now) {
                admitted += 1;
            }
        }
    }
    assert_eq!(admitted, NEW_BINDING_BURST_PER_END_SITE as usize);

    // Another end site keeps its own burst.
    let other = source_in_site_two();
    for _ in 0..NEW_BINDING_BURST_PER_END_SITE as usize {
        assert!(limiter.try_take_at(other, now));
    }
    assert!(!limiter.try_take_at(other, now));

    // The end site earns one binding per ten seconds (0.1 per second).
    let later = now + Duration::from_secs(10);
    assert!(limiter.try_take_at(source_in_site_one(0), later));
    assert!(!limiter.try_take_at(source_in_site_one(1), later));
}
