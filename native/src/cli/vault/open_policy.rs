//! Read/set/revoke the two independent opening consents.
use clap::Args;
use super::manage::SiteArg;
use super::output::{key_lines, pretty};

#[derive(Args)]
pub(super) struct PolicyArgs {
    #[command(flatten)]
    site: SiteArg,
    #[arg(long, help = "Trust this exact vault/backend/account/root for mounting (true/false)")]
    trust_root: Option<bool>,
    #[arg(long, help = "Allow a cleartext app-data copy when the mount runtime is absent (true/false)")]
    always_copy: Option<bool>,
    #[arg(long)]
    json: bool,
}

pub(super) fn run(args: PolicyArgs) -> Result<i32, String> {
    if let Some(trusted) = args.trust_root {
        crate::vault::set_root_trust(&args.site.site, trusted)?;
    }
    if let Some(always) = args.always_copy {
        crate::vault::set_cleartext_copy_policy(&args.site.site, always)?;
    }
    let record = crate::vault::RegistryStore::open_default().resolve(&args.site.site)?;
    let stored = crate::vault::stored_policy(&record)?;
    let resolved = crate::vault::mount_policy(&record.id);
    let value = serde_json::json!({
        "site": record.id,
        "stored": stored,
        "current": resolved.as_ref().ok(),
        "resolution_error": resolved.err(),
    });
    if args.json { println!("{}", pretty(&value)?); }
    else { for line in key_lines(&value) { println!("{line}"); } }
    Ok(0)
}
