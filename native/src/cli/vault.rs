//! `se vault`: Cryptomator vaults on the command line (vault plan C7.3,
//! VLT-M9, U26-10): list, show, detect, add, create and remove entries of the
//! vault list, unlock and lock them in the background worker, change the
//! password and the lock settings, keep vault sync jobs (`sync add`, `sync
//! run`), compare their two sides (`compare`, `resolve`) and check a site
//! (`check`). Mounting a vault as a drive is `se drive mount vault://<SITE>`.
//! Exit codes: 0 done, 1 failed (`se: <message>` on stderr), 2 wrong call;
//! `check` also answers 1 when it found something. Every command with
//! `--json` prints the keys of C7.3; no password ever stands in an argument.
use clap::{Args, Subcommand};

#[path = "vault/compare.rs"]
mod compare;
#[path = "vault/complete.rs"]
mod complete;
#[path = "vault/manage.rs"]
mod manage;
#[path = "vault/output.rs"]
pub(crate) mod output;
#[path = "vault/sync.rs"]
mod sync;
#[path = "vault/unlock.rs"]
mod unlock;

const VAULT_HELP: &str = "\
Cryptomator vaults: the same vault list and sessions as the GUI.

Examples:
  se vault list
  se vault unlock Private --password-stdin < password.txt
  se vault lock --all
  se vault create ~/Vaults/Work --label Work --password-stdin
  se vault sync add Work --with /mnt/usb/work-copy
  se vault sync run 18a2f0c
  se vault compare 18a2f0c
  se vault resolve 18a2f0c 4f1c0a9b2e7d3a55 --action keep-a
  se vault check Work
  se drive mount vault://Work

A password comes from standard input (`--password-stdin`, read to the end) or
from the remembered one (`--saved-password`). Recovery keys are not offered:
Cryptomator Desktop creates and uses them.";

#[derive(Args)]
#[command(long_about = VAULT_HELP)]
pub(super) struct VaultArgs {
    #[command(subcommand)]
    command: Option<VaultCommand>,
}

#[derive(Subcommand)]
enum VaultCommand {
    #[command(about = "List the entered vaults with their state")]
    List {
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Show every setting and the state of one vault")]
    Show(manage::SiteJsonArgs),
    #[command(about = "Tell whether a location holds a Cryptomator vault")]
    Detect(manage::DetectArgs),
    #[command(about = "Enter an existing vault into the vault list")]
    Add(manage::AddArgs),
    #[command(about = "Create a new vault and enter it")]
    Create(manage::CreateArgs),
    #[command(about = "Remove a vault from the vault list (its files stay)")]
    Remove(manage::SiteArg),
    #[command(about = "Unlock a vault in the background worker")]
    Unlock(unlock::UnlockArgs),
    #[command(about = "Lock one vault or every vault")]
    Lock(unlock::LockArgs),
    #[command(about = "Change the password of a vault")]
    Password(manage::PasswordArgs),
    #[command(about = "Forget the remembered password of a vault")]
    ForgetPassword(manage::SiteArg),
    #[command(about = "Change the name and the lock settings of a vault")]
    Settings(manage::SettingsArgs),
    #[command(about = "Check the structure of an unlocked vault")]
    Check(compare::CheckArgs),
    #[command(about = "Vault sync jobs: create and run")]
    Sync(sync::SyncArgs),
    #[command(about = "Compare the two sides of a vault sync job")]
    Compare(compare::CompareArgs),
    #[command(about = "Resolve one entry of a comparison, or every conflict")]
    Resolve(compare::ResolveArgs),
}

pub(super) fn run(args: VaultArgs) -> Result<i32, String> {
    match args.command.unwrap_or(VaultCommand::List { json: false }) {
        VaultCommand::List { json } => manage::list(json),
        VaultCommand::Show(args) => manage::show(args),
        VaultCommand::Detect(args) => manage::detect(args),
        VaultCommand::Add(args) => manage::add(args),
        VaultCommand::Create(args) => manage::create(args),
        VaultCommand::Remove(args) => manage::remove(args),
        VaultCommand::Unlock(args) => unlock::unlock(args),
        VaultCommand::Lock(args) => unlock::lock(args),
        VaultCommand::Password(args) => manage::password(args),
        VaultCommand::ForgetPassword(args) => manage::forget_password(args),
        VaultCommand::Settings(args) => manage::settings(args),
        VaultCommand::Check(args) => compare::check(args),
        VaultCommand::Sync(args) => sync::run(args),
        VaultCommand::Compare(args) => compare::compare(args),
        VaultCommand::Resolve(args) => compare::resolve(args),
    }
}

// Stage tests that run the built `se` (VLT-M9, VLT-M14): Linux only, and
// `#[ignore]`d until their stage gives them `SMART_EXPLORER_VAULT_SE_BIN`.
#[cfg(all(test, target_os = "linux"))]
#[path = "vault/task_cli_tests.rs"]
mod task_cli_tests;
#[cfg(all(test, target_os = "linux"))]
#[path = "vault/task_env.rs"]
pub(crate) mod task_env;
#[cfg(all(test, target_os = "linux"))]
#[path = "vault/task_flow.rs"]
mod task_flow;
#[cfg(all(test, target_os = "linux"))]
#[path = "vault/task_json_tests.rs"]
mod task_json_tests;
#[cfg(all(test, target_os = "linux"))]
#[path = "vault/task_klartext_tests.rs"]
mod task_klartext_tests;
#[cfg(all(test, target_os = "linux"))]
#[path = "vault/task_scan.rs"]
mod task_scan;
#[cfg(test)]
#[path = "vault/tests.rs"]
mod tests;
