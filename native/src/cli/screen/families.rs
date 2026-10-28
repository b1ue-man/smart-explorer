//! Generation-bearing sources for separate native window viewers. The
//! listing comes from the background service's device feed when it holds
//! one for the device (design 5.2: no extra request, neither se window has
//! to run); otherwise the device is asked once as before.
use crate::screen::core::family_protocol::FamilyListing;
use clap::Args;

#[derive(Args, Debug, PartialEq, Eq)]
pub(super) struct FamiliesArgs {
    device: String,
    #[arg(long)]
    json: bool,
}

/// The feed's listing of the device `target` reaches, if the background
/// service holds one.
pub(crate) fn feed_listing(
    snapshot: &crate::daemon::ShareWorkerSnapshot,
    target: &crate::share::PeerOpenTarget,
) -> Option<FamilyListing> {
    let (device, key) = crate::desktop::core::tiles::identity_of(&snapshot.profiles, target)?;
    crate::desktop::core::tiles::families_of(snapshot, &device, &key)
}

pub(super) fn run(args: FamiliesArgs) -> Result<i32, String> {
    let choice = super::select_device(&args.device)?;
    let fed = crate::daemon::share_worker_snapshot()
        .ok()
        .and_then(|snapshot| feed_listing(&snapshot, &choice.target));
    let listing = match fed {
        Some(listing) => listing,
        None => {
            let notice = super::pending_notice(choice.name);
            crate::daemon::screen_families(choice.target, None, notice)?
        }
    };
    let value = serde_json::to_value(&listing).map_err(|error| error.to_string())?;
    super::output::print(args.json, &value, || {
        let clean = |s: &str| s.replace(['\t', '\r', '\n'], " ");
        let mut rows: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| {
                format!(
                    "window\t{}\t{}\t{}\t{}\n",
                    entry.source,
                    entry.identity.generation,
                    clean(&entry.window.app),
                    clean(&entry.window.title)
                )
            })
            .collect();
        rows.extend(
            listing
                .limitations
                .iter()
                .map(|reason| format!("limit\t{}\n", clean(reason))),
        );
        rows.concat()
    });
    Ok(0)
}
