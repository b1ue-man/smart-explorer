use crossbeam_channel::Sender;

use super::archive::{archive_binary, is_auto_update_paused, resume_auto_update};
use super::config::{last_applied_path, update_source_str};
use super::core::parse_ver;
use super::feed::classify_feed;
use super::os::{apply_via_installed_updater, ensure_installed_updater};
use super::types::UpdateMsg;

/// Force a forward update to the feed's latest, clearing any rollback pin.
/// Runs on its own thread; result via `tx`.
pub fn update_to_latest_async(tx: Sender<UpdateMsg>) {
    std::thread::Builder::new()
        .name("update-latest".into())
        .spawn(move || {
            resume_auto_update();
            match check_and_apply(true) {
                Ok(Some(msg)) => {
                    let _ = tx.send(msg);
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = tx.send(UpdateMsg::Error(e));
                }
            }
        })
        .ok();
}

/// Check the feed and, if it carries a newer version, swap the binary in
/// place. Runs on its own thread; result arrives via `tx`.
/// `manual` = user clicked "check now" (gets feedback even for no-op results).
pub fn check_async(tx: Sender<UpdateMsg>, manual: bool) {
    std::thread::Builder::new()
        .name("updater".into())
        .spawn(move || {
            let result = check_and_apply(manual);
            match result {
                Ok(Some(msg)) => {
                    let _ = tx.send(msg);
                }
                Ok(None) => {}
                Err(e) => {
                    if manual {
                        let _ = tx.send(UpdateMsg::Error(e));
                    }
                }
            }
        })
        .ok();
}

fn check_and_apply(manual: bool) -> Result<Option<UpdateMsg>, String> {
    if !manual && is_auto_update_paused() {
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
        if parse_ver(&feed_version) == parse_ver(current) {
            let _ = ensure_installed_updater(&feed, &feed_version, false);
        }
        return Ok(if manual {
            Some(UpdateMsg::UpToDate { feed_version })
        } else {
            None
        });
    }

    if let Ok(last) = std::fs::read_to_string(last_applied_path()) {
        if last.trim() == feed_version {
            return Err(format!(
                "Update {} wurde bereits angewendet, aber die Programmversion ist weiterhin {} — version.txt im Feed passt nicht zur EXE",
                feed_version, current
            ));
        }
    }

    let new_exe = feed.fetch_exe(&feed_version)?;
    archive_binary(current);
    let helper = ensure_installed_updater(&feed, &feed_version, true)?;
    apply_via_installed_updater(&helper, &new_exe, &feed_version)?;
    resume_auto_update();
    Ok(Some(UpdateMsg::AppliedViaWorker {
        version: feed_version.clone(),
    }))
}
