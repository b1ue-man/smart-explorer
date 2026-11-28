//! `se elevated …` (hidden): the helper of the elevated commands
//! (`engine::elevate`, plan docs/plaene/2026-11-28-ersetzen-masken-shell,
//! contract 3). se starts it itself; it is no command for people.
//!
//! - `exec`, `confirm`, `run`: `crate::elevate::run_helper` (the elevated
//!   side, the confirmation proof, the Share host's client wrapper).
//! - `managed-hooks …`: the managed hook installation of the agent hosts,
//!   which runs elevated (`hook::hook_managed::run_elevated`).
//! - `ask-password --caption C --message M`: the password window of the
//!   `shell` and `exec` tools for `sudo` on SSH remotes (`exec_password.rs`);
//!   prints the password on standard output, exit 1 when cancelled.
#[path = "exec_password.rs"]
pub(super) mod password;

#[derive(clap::Args, Debug)]
pub(super) struct ElevatedArgs {
    /// The helper's verb and its arguments, passed on unchanged.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
    pub(super) args: Vec<String>,
}

pub(super) fn run(args: ElevatedArgs) -> Result<i32, String> {
    match args.args.split_first() {
        Some((verb, rest)) if verb == "managed-hooks" => {
            super::hook::hook_managed::run_elevated(rest)
        }
        Some((verb, rest)) if verb == "ask-password" => password::run_window(rest),
        _ => crate::elevate::run_helper(&args.args),
    }
}
