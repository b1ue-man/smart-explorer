use crossbeam_channel::Sender;
use serde::Serialize;

use super::archive::pinned_version_checked;
use super::config::update_source_str;
use super::core::parse_ver;
use super::feed::classify_feed;
use super::feed_files::read_feed_version;
use super::feed_trust::{read_trust, UnsignedConsent};
use super::staging::stage_with_trust;
use super::types::UpdateMsg;

/// Force a forward check to the feed's latest even while rollback-pinned.
/// The worker only stages verified payloads; the pin remains untouched until
/// the user explicitly applies the staged bundle.
/// Runs on its own thread; result via `tx`.
pub fn update_to_latest_async(tx: Sender<UpdateMsg>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("update-latest".into())
        .spawn(
            move || match check_and_stage(true, &UnsignedConsent::none()) {
                Ok(Some(msg)) => {
                    let _ = tx.send(msg);
                }
                Ok(None) => {
                    let _ = tx.send(UpdateMsg::Finished);
                }
                Err(e) => {
                    let _ = tx.send(UpdateMsg::Error(e));
                }
            },
        )
        .map(|_| ())
}

/// Check the feed and, if it carries a newer version, download and verify all
/// release payloads. Installed files and processes are never touched here.
/// `manual` = user clicked "check now" (gets feedback even for no-op results).
pub fn check_async(tx: Sender<UpdateMsg>, manual: bool) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("updater".into())
        .spawn(move || {
            let result = check_and_stage(manual, &UnsignedConsent::none());
            match result {
                Ok(Some(msg)) => {
                    let _ = tx.send(msg);
                }
                Ok(None) => {
                    let _ = tx.send(UpdateMsg::Finished);
                }
                Err(e) => {
                    if manual {
                        let _ = tx.send(UpdateMsg::Error(e));
                    } else {
                        let _ = tx.send(UpdateMsg::BackgroundError(e));
                    }
                }
            }
        })
        .map(|_| ())
}

/// The user confirmed staging the unsigned `version` of the configured feed
/// (answer to `UpdateMsg::UnsignedConsentNeeded`). The feed is read again:
/// the consent covers only `version`, so a feed that moved on to another
/// unsigned version asks again. Like "Auf neueste aktualisieren", a
/// rollback pin does not hold this explicit request back.
pub fn stage_confirmed_async(tx: Sender<UpdateMsg>, version: String) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("update-confirmed".into())
        .spawn(move || {
            let consent = UnsignedConsent::for_version(&version);
            let _ = tx.send(match check_and_stage(true, &consent) {
                Ok(Some(msg)) => msg,
                Ok(None) => UpdateMsg::Finished,
                Err(e) => UpdateMsg::Error(e),
            });
        })
        .map(|_| ())
}

fn check_and_stage(manual: bool, consent: &UnsignedConsent) -> Result<Option<UpdateMsg>, String> {
    if !manual && pinned_version_checked()?.is_some() {
        return Ok(None);
    }
    let raw = match update_source_str() {
        Some(s) => s,
        None => {
            return Ok(if manual {
                Some(UpdateMsg::NoFeed)
            } else {
                None
            });
        }
    };
    let feed = classify_feed(&raw);

    let feed_version = feed.read_version()?;
    if feed_version.is_empty() {
        return Err(format!("version.txt im Feed {} ist leer", feed.display()));
    }

    let current = env!("CARGO_PKG_VERSION");
    if parse_ver(&feed_version) <= parse_ver(current) {
        return Ok(if manual {
            Some(UpdateMsg::UpToDate { feed_version })
        } else {
            None
        });
    }

    // E12: a newer release without valid signature that needs consent is
    // only reported; nothing is downloaded before the user confirms.
    let feed = feed.pinned(&feed_version);
    let trust = read_trust(&feed, &feed_version)?;
    if let Some(kind) = trust.missing_consent(&feed_version, consent) {
        return Ok(Some(UpdateMsg::UnsignedConsentNeeded {
            version: feed_version,
            feed: kind,
        }));
    }
    let bundle = stage_with_trust(&feed, &feed_version, &trust)?;
    Ok(Some(UpdateMsg::Staged(bundle)))
}

#[derive(Clone, Debug, Serialize)]
pub struct FeedCheck {
    pub source: String,
    pub current_version: String,
    pub feed_version: String,
    pub update_available: bool,
}

/// The configured feed's version compared with this build; `None` without
/// a configured update source.
pub fn check_feed() -> Result<Option<FeedCheck>, String> {
    let Some(source) = update_source_str() else {
        return Ok(None);
    };
    let feed_version = read_feed_version(&source)?;
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let update_available = parse_ver(&feed_version) > parse_ver(&current_version);
    Ok(Some(FeedCheck {
        source,
        current_version,
        feed_version,
        update_available,
    }))
}

#[cfg(test)]
#[path = "flow_unsigned_tests.rs"]
mod unsigned_tests;
