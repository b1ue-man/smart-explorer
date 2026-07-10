use crossbeam_channel::Sender;

use super::archive::pinned_version_checked;
use super::config::update_source_str;
use super::core::parse_ver;
use super::feed::classify_feed;
use super::staging::stage_from_feed;
use super::types::UpdateMsg;

/// Force a forward check to the feed's latest even while rollback-pinned.
/// The worker only stages verified payloads; the pin remains untouched until
/// the user explicitly applies the staged bundle.
/// Runs on its own thread; result via `tx`.
pub fn update_to_latest_async(tx: Sender<UpdateMsg>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("update-latest".into())
        .spawn(move || match check_and_stage(true) {
            Ok(Some(msg)) => {
                let _ = tx.send(msg);
            }
            Ok(None) => {
                let _ = tx.send(UpdateMsg::Finished);
            }
            Err(e) => {
                let _ = tx.send(UpdateMsg::Error(e));
            }
        })
        .map(|_| ())
}

/// Check the feed and, if it carries a newer version, download and verify all
/// release payloads. Installed files and processes are never touched here.
/// `manual` = user clicked "check now" (gets feedback even for no-op results).
pub fn check_async(tx: Sender<UpdateMsg>, manual: bool) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("updater".into())
        .spawn(move || {
            let result = check_and_stage(manual);
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

fn check_and_stage(manual: bool) -> Result<Option<UpdateMsg>, String> {
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

    let bundle = stage_from_feed(&feed, &feed_version)?;
    Ok(Some(UpdateMsg::Staged(bundle)))
}
