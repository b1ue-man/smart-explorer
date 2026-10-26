//! Generation-bearing sources for separate native window viewers.
use clap::Args;

#[derive(Args, Debug, PartialEq, Eq)]
pub(super) struct FamiliesArgs {
    device: String,
    #[arg(long)]
    json: bool,
}

pub(super) fn run(args: FamiliesArgs) -> Result<i32, String> {
    let choice = super::select_device(&args.device)?;
    let listing = crate::daemon::screen_families(choice.target, None, super::pending_notice(choice.name))?;
    let value = serde_json::to_value(&listing).map_err(|error|error.to_string())?;
    super::output::print(args.json, &value, || {
        let clean = |s:&str|s.replace(['\t','\r','\n']," ");
        let mut rows:Vec<_> = listing.entries.iter().map(|entry|format!("window\t{}\t{}\t{}\t{}\n",
            entry.source,entry.identity.generation,clean(&entry.window.app),clean(&entry.window.title))).collect();
        rows.extend(listing.limitations.iter().map(|reason|format!("limit\t{}\n",clean(reason))));
        rows.concat()
    });
    Ok(0)
}
