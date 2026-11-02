//! `se share request policy`: whether requests carrying this device's
//! direct code from devices without an authorization are accepted
//! automatically – the setting of the desktop page „Anfragen“ and of
//! Android `share.setPolicy` (`share_direct_policy.json`, read at each
//! decision). With `auto-accept`, `--preset` and the rights options set the
//! rights such devices get (F6, the rights of the Direct code shared with
//! `se share identity`); without them they connect without rights and wait
//! for a confirmation.
use clap::{Args, ValueEnum};

#[derive(Args)]
pub(super) struct PolicyArgs {
    #[arg(value_enum, help = "New policy; omit to show the current one")]
    policy: Option<PolicyChoice>,
    // Rights of devices that connect with this direct code (also used by auto-accept)
    #[command(flatten)]
    rights: super::rights_args::RightsArgs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum PolicyChoice {
    /// Every new device waits for an explicit decision
    Ask,
    /// Devices with this direct code connect without a decision and get the
    /// direct code's rights (--preset; default none, they then wait for a
    /// confirmation); removed identities stay blocked
    AutoAccept,
}

impl From<PolicyChoice> for crate::share::DirectRequestPolicy {
    fn from(choice: PolicyChoice) -> Self {
        match choice {
            PolicyChoice::Ask => Self::Ask,
            PolicyChoice::AutoAccept => Self::AutoAccept,
        }
    }
}

pub(super) fn run(args: PolicyArgs, json: bool) -> Result<(), String> {
    if args.rights.given() {
        super::identity_command::store_code_rights(&args.rights)?;
    }
    if let Some(choice) = args.policy {
        crate::share::DirectRequestPolicy::from(choice).save()?;
    }
    let loaded = crate::share::DirectRequestPolicy::load();
    let current = loaded.as_ref().copied().unwrap_or_default();
    let name = match current {
        crate::share::DirectRequestPolicy::Ask => "ask",
        crate::share::DirectRequestPolicy::AutoAccept => "auto-accept",
    };
    let code_rights = super::checked_profiles()?.code_rights;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "request_policy": name, "error": loaded.as_ref().err(),
                "code_rights": code_rights.as_ref().map(super::rights_args::rights_value),
            })
        );
    } else {
        println!("request_policy\t{name}");
        println!(
            "{}",
            super::identity_command::code_rights_line(code_rights.as_ref())
        );
        if let Err(error) = &loaded {
            println!("request_policy_error\t{error}\tdecisions use `ask`");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn luecken_task_request_policy_parses_ask_and_auto_accept_only() {
        for command in [
            vec!["se", "share", "request", "policy"],
            vec!["se", "share", "request", "policy", "ask"],
            vec!["se", "share", "request", "policy", "auto-accept", "--json"],
        ] {
            assert!(
                crate::cli::Cli::try_parse_from(command.clone()).is_ok(),
                "{command:?}"
            );
        }
        assert!(
            crate::cli::Cli::try_parse_from(["se", "share", "request", "policy", "always"])
                .is_err()
        );
        assert_eq!(
            crate::share::DirectRequestPolicy::from(super::PolicyChoice::AutoAccept),
            crate::share::DirectRequestPolicy::AutoAccept
        );
    }
}
