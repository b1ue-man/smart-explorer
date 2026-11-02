//! `se vault unlock` and `se vault lock` (vault plan C7.3, U26-11): the
//! sessions of the unlocked vaults live in the background worker, so both
//! ask it (unlocking starts it). A password comes from standard input or from
//! the stored one (`--saved-password`), never from an argument or a prompt.
use clap::Args;
use clap_complete::ArgValueCandidates;
use serde_json::json;

use super::complete::site_candidates;
use super::output::{clean, pretty, site_of, vault_error};

#[derive(Args)]
pub(super) struct UnlockArgs {
    #[arg(
        value_name = "SITE",
        help = "Vault ID, ID prefix (at least 4 digits) or label from `se vault list`",
        add = ArgValueCandidates::new(site_candidates)
    )]
    site: String,
    #[arg(
        long,
        conflicts_with = "saved_password",
        help = "Read the password from standard input (to the end, trailing line break removed)"
    )]
    password_stdin: bool,
    #[arg(long, help = "Use the password remembered for this vault")]
    saved_password: bool,
    #[arg(
        long,
        value_name = "true|false",
        requires = "password_stdin",
        // Remembering a password needs a password given now: with the stored
        // one the option is refused at parsing (the run of 2026-10-21 let
        // `--saved-password --remember true` pass with `requires` alone).
        conflicts_with = "saved_password",
        help = "Remember (true) or forget (false) the password after a successful unlock"
    )]
    remember: Option<bool>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
pub(super) struct LockArgs {
    #[arg(
        value_name = "SITE",
        help = "Vault ID, ID prefix (at least 4 digits) or label from `se vault list`",
        add = ArgValueCandidates::new(site_candidates),
        conflicts_with = "all",
        required_unless_present = "all"
    )]
    site: Option<String>,
    #[arg(long, help = "Lock every unlocked vault")]
    all: bool,
    #[arg(
        long,
        help = "Lock although drive changes are unsaved; they stay encrypted in the cache"
    )]
    keep_unsaved: bool,
}

pub(super) fn unlock(args: UnlockArgs) -> Result<i32, String> {
    if !args.password_stdin && !args.saved_password {
        return Err("Passwort fehlt: --password-stdin oder --saved-password".into());
    }
    let record = site_of(&args.site)?;
    let result = if args.saved_password {
        crate::daemon::unlock_vault_saved(&record.id)
    } else {
        let password = crate::cli::setup::read_stdin_secret()?;
        crate::daemon::unlock_vault(&record.id, &password, args.remember)
    };
    let unlocked = result.map_err(|error| vault_error(&error, &record.id))?;
    if let Some(problem) = &unlocked.remember_error {
        eprintln!("se: Passwort nicht gemerkt: {}", clean(problem));
    }
    if args.json {
        println!(
            "{}",
            pretty(&json!({ "site": record.id, "state": "unlocked" }))?
        );
    } else {
        println!("unlocked\t{}", record.id);
    }
    Ok(0)
}

pub(super) fn lock(args: LockArgs) -> Result<i32, String> {
    let record = args.site.as_deref().map(site_of).transpose()?;
    let site = record.as_ref().map(|record| record.id.as_str());
    let locked = crate::daemon::lock_vaults(site, args.keep_unsaved).map_err(|error| {
        let mut text = vault_error(&error, site.unwrap_or("--all"));
        if error.code.as_deref() == Some("vault_unsaved_changes") {
            text.push_str(" --keep-unsaved bewahrt die Änderungen verschlüsselt auf.");
        }
        text
    })?;
    if locked.is_empty() {
        println!("locked\t0");
    }
    for site in locked {
        println!("locked\t{site}");
    }
    Ok(0)
}
