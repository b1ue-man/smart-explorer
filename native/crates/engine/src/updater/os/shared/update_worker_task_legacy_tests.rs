//! E25: the earlier callers refuse an unsigned `http://` feed. Signed feeds
//! and unsigned `https://` feeds keep their earlier checks.
use super::{refuse_unsigned_plain_http, Feed};
use crate::updater::feed_trust::tests::fixture::{AppData, ReleaseFolder, TestKey, VERSION};
use crate::updater::feed_trust::{read_trust, FeedTrust};
use crate::updater::UnsignedFeed;

fn plain_http_feed() -> Feed {
    Feed::Http("http://feed.invalid/smart-explorer".into())
}

fn https_feed() -> Feed {
    Feed::Http("https://feed.invalid/smart-explorer".into())
}

#[test]
fn update_worker_task_legacy_refuses_an_unsigned_plain_http_feed() {
    let trust = FeedTrust::Unsigned(UnsignedFeed::PlainHttp);
    let error = refuse_unsigned_plain_http(&plain_http_feed(), &trust)
        .expect_err("an unsigned http:// feed is refused");
    assert!(error.starts_with("legacy_http_unsigned_refused"), "{error}");
}

#[test]
fn update_worker_task_legacy_keeps_unsigned_https_feeds() {
    for kind in [UnsignedFeed::Custom, UnsignedFeed::OfficialRelease] {
        let trust = FeedTrust::Unsigned(kind);
        refuse_unsigned_plain_http(&https_feed(), &trust)
            .expect("an unsigned https:// feed is not refused here");
    }
}

#[test]
fn update_worker_task_legacy_keeps_signed_plain_http_feeds() {
    let _app = AppData::new();
    let key = TestKey::install();
    let release = ReleaseFolder::new(VERSION);
    release.sign(&key);
    let trust = read_trust(&release.feed(), VERSION).expect("the signed release");
    assert!(trust.manifest().is_some());
    refuse_unsigned_plain_http(&plain_http_feed(), &trust)
        .expect("a signed http:// feed is not refused here");
}
