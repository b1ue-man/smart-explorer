//! `se share exec accounts` (PRM-M8 accounts, D.2.4): the JSON form of the
//! account choices, the text lines, and exactly the words of the setup
//! modes. Runs on Linux and Windows without an account service.
use clap::{FromArgMatches, Parser};
use serde_json::json;

use super::*;
use crate::share::exec_helper_protocol::{AccountChoiceProfile, AccountReason};

#[derive(Parser)]
struct Accounts {
    #[command(flatten)]
    args: AccountsArgs,
}

fn parsed(words: &[&str]) -> Result<(ExecAccountsAction, bool), String> {
    let arguments = std::iter::once("accounts").chain(words.iter().copied());
    let matches = <Accounts as clap::CommandFactory>::command()
        .try_get_matches_from(arguments)
        .map_err(|error| error.to_string())?;
    let accounts = Accounts::from_arg_matches(&matches).map_err(|error| error.to_string())?;
    let command = accounts.args.command.ok_or("no operation")?;
    Ok(action_of(command))
}

fn choices() -> AccountChoices {
    let mut remote = AccountChoice::other(AccountChoiceProfile::Remote, "se-remote");
    remote.refuse(
        AccountReason::HelperNotInstalled,
        "the account service is not set up on this host".into(),
        Some("as administrator: se share exec accounts setup --remote-account".into()),
    );
    let mut deploy = AccountChoice::other(AccountChoiceProfile::Account, "deploy");
    deploy.admin_group = Some(true);
    AccountChoices {
        helper: HelperStatus {
            state: HelperState::NotInstalled,
            version: None,
            detail: None,
        },
        choices: vec![AccountChoice::own("alice"), remote, deploy],
    }
}

#[test]
fn perm_task_cli_accounts_json() {
    let value = value(&choices());
    assert_eq!(
        value["helper"],
        json!({"state": "not_installed", "version": null, "detail": null})
    );
    assert_eq!(
        value["choices"][0],
        json!({"profile": "own", "label": "alice", "available": true})
    );
    let remote = &value["choices"][1];
    assert_eq!(remote["profile"], "remote");
    assert_eq!(remote["account"], "se-remote");
    assert_eq!(remote["available"], false);
    assert_eq!(remote["reason_code"], "helper_not_installed");
    assert!(remote["reason"].as_str().unwrap().contains("not set up"));
    assert!(remote["next_step"].as_str().unwrap().contains("setup"));
    assert_eq!(
        (&remote["elevated"], &remote["admin_group"]),
        (&json!(false), &json!(false))
    );
    let deploy = &value["choices"][2];
    assert_eq!(deploy["profile"], "account");
    assert_eq!(deploy["account"], "deploy");
    assert_eq!(deploy["available"], true);
    assert_eq!(deploy["admin_group"], true);
    assert!(deploy.get("reason").is_none(), "{deploy}");

    let lines = lines(&choices());
    assert_eq!(lines[0], "helper\tstate=not_installed\tversion=-\tdetail=-");
    assert_eq!(lines[1], "account\town\tlabel=alice\tavailable=true");
    assert!(lines[2].starts_with("account\tremote\taccount=se-remote\tavailable=false\treason="));
    assert!(lines[2].contains("\treason_code=helper_not_installed\tnext_step="));
    assert!(lines[2].ends_with("\televated=false\tadmin_group=false"));
    assert_eq!(
        lines.last().unwrap(),
        "setup\tse share exec accounts setup --remote-account"
    );

    // The words of the setup modes, and `--yes` only with setup (D.2.4).
    assert_eq!(
        parsed(&["setup", "--remote-account", "--allow", "deploy", "--yes"]).unwrap(),
        (
            ExecAccountsAction::Setup {
                remote_account: true,
                allow: vec!["deploy".into()],
            },
            true
        )
    );
    assert_eq!(
        parsed(&["allow", "deploy"]).unwrap(),
        (
            ExecAccountsAction::Allow {
                account: "deploy".into()
            },
            false
        )
    );
    assert_eq!(
        parsed(&["disallow", "deploy"]).unwrap().0,
        ExecAccountsAction::Disallow {
            account: "deploy".into()
        }
    );
    assert_eq!(parsed(&["update"]).unwrap().0, ExecAccountsAction::Update);
    assert_eq!(
        parsed(&["remove", "--delete-remote-account"]).unwrap().0,
        ExecAccountsAction::Remove {
            delete_remote_account: true
        }
    );
    for words in [
        &["allow", "deploy", "--yes"][..],
        &["update", "--yes"],
        &["remove", "--yes"],
        &["remove", "--allow", "deploy"],
    ] {
        assert!(parsed(words).is_err(), "{words:?}");
    }
    // The whole command line, as `se share exec accounts` gets it.
    for words in [
        &["se", "share", "exec", "accounts", "--json"][..],
        &[
            "se",
            "share",
            "exec",
            "accounts",
            "setup",
            "--remote-account",
            "--yes",
        ],
        &[
            "se",
            "share",
            "exec",
            "accounts",
            "remove",
            "--delete-remote-account",
        ],
    ] {
        assert!(crate::cli::Cli::try_parse_from(words).is_ok(), "{words:?}");
    }
    // The MCP tool `se` lists accounts but never sets them up (C7).
    let mcp = |words: &[&str]| {
        let words: Vec<String> = words.iter().map(|word| word.to_string()).collect();
        crate::cli::share::exec_approvals::mcp_refusal(&words)
    };
    assert!(mcp(&["share", "exec", "accounts", "--json"]).is_none());
    assert!(mcp(&["share", "exec", "accounts", "setup", "--yes"]).is_some());
    assert!(mcp(&["share", "exec", "accounts", "allow", "deploy"]).is_some());
}
