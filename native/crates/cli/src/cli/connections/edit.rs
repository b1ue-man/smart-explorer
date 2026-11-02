//! `se connections edit`: change a saved remote like "Bearbeiten" in the
//! Android connection list and the desktop dialog "Verbindung bearbeiten".
//! Options not given keep their saved value; the change goes through
//! `connect::save_edited_connection`, the path both GUIs use.
use clap::Args;
use clap_complete::ArgValueCandidates;

use crate::creds::AuthKind;

use super::setup;

const EDIT_HELP: &str = "\
Change a saved remote. Options not given keep their saved value; an empty
password keeps the stored one. Changing host, port, user or root moves the
entry: its stored password or key passphrase moves along unless the login kind
changes (a password is never used as a key passphrase). Sync setups and drives
that still use the old address are named in a warning and left unchanged.

Examples:
  se connections edit prod --host prod.example.com
  se connections edit prod --key ~/.ssh/id_ed25519 --password-stdin
  se connections edit prod --ssh-agent
  se connections edit nas --root /backup/2026 --label nas-2026
  se connections edit prod --no-agent";

#[derive(Args)]
pub(super) struct RemoteSelectorArgs {
    #[arg(
        allow_hyphen_values = true,
        help = "Exact saved label or account from `se connections list`",
        add = ArgValueCandidates::new(remote_candidates)
    )]
    pub(super) selector: String,
}

#[derive(Args)]
#[command(long_about = EDIT_HELP)]
pub(super) struct EditArgs {
    #[command(flatten)]
    remote: RemoteSelectorArgs,
    #[arg(long, help = "New host")]
    host: Option<String>,
    #[arg(long, help = "New port")]
    port: Option<u16>,
    #[arg(long, help = "New login user (DOMAIN\\user for SMB and UNC shares)")]
    user: Option<String>,
    #[arg(long, help = "New start path; smb: /<share>/<folder>; share: UNC path")]
    root: Option<String>,
    #[arg(long, help = "New label for @label:/path")]
    label: Option<String>,
    #[arg(
        long,
        conflicts_with_all = ["ssh_agent", "password_auth"],
        help = "SFTP: log in with this private key file"
    )]
    key: Option<String>,
    #[arg(
        long,
        conflicts_with_all = ["password_auth", "password", "password_stdin"],
        help = "SFTP: log in with the running SSH agent; stores no secret"
    )]
    ssh_agent: bool,
    #[arg(
        long,
        help = "Log in with a password again (instead of a key or the SSH agent)"
    )]
    password_auth: bool,
    #[arg(
        long,
        conflicts_with = "no_agent",
        help = "SFTP: deploy and use the Smart Explorer acceleration agent"
    )]
    agent: bool,
    #[arg(long, help = "SFTP: stop using the acceleration agent")]
    no_agent: bool,
    #[arg(
        long,
        conflicts_with = "password_stdin",
        help = "New password or key passphrase to store"
    )]
    password: Option<String>,
    #[arg(long, help = "Read the new password or key passphrase from stdin")]
    password_stdin: bool,
}

pub(super) fn run(args: EditArgs) -> Result<String, String> {
    let previous = setup::find_saved_remote(&args.remote.selector)?;
    let secret = if args.password_stdin {
        Some(setup::read_stdin_secret()?)
    } else {
        args.password
    };
    let (key, ssh_agent) = match (args.key, args.ssh_agent, args.password_auth) {
        (Some(key), _, _) => (Some(key), false),
        (None, true, _) => (None, true),
        (None, false, true) => (None, false),
        (None, false, false) => match &previous.auth {
            AuthKind::Key { path } => (Some(path.clone()), false),
            AuthKind::SshAgent => (None, true),
            AuthKind::Password => (None, false),
        },
    };
    let use_agent = if args.agent {
        true
    } else if args.no_agent {
        false
    } else {
        previous.use_agent
    };
    // A UNC share has no port of its own; the builder refuses a given one.
    let port = if previous.protocol == crate::creds::Protocol::Share {
        args.port
    } else {
        Some(args.port.unwrap_or(previous.port))
    };
    let input = setup::RemoteConnectionInput {
        protocol: previous.protocol,
        host: Some(args.host.unwrap_or_else(|| previous.host.clone())),
        port,
        user: args.user.unwrap_or_else(|| previous.user.clone()),
        root: args.root.unwrap_or_else(|| previous.root.clone()),
        label: args.label.unwrap_or_else(|| previous.label.clone()),
        key,
        use_agent,
        ssh_agent,
        secret,
    };
    setup::edit_remote(&previous, input)
}

fn remote_candidates() -> Vec<clap_complete::CompletionCandidate> {
    crate::creds::load_connections_checked()
        .unwrap_or_default()
        .into_iter()
        .map(|connection| {
            let help = format!(
                "{} · {}",
                connection.protocol.as_str(),
                connection.account()
            )
            .replace(['\t', '\r', '\n'], " ");
            clap_complete::CompletionCandidate::new(connection.display())
                .help(Some(clap::builder::StyledStr::from(help)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::super::super::{Cli, Command as RootCommand};

    fn parses(arguments: &[&str]) -> bool {
        let mut all = vec!["se", "connections"];
        all.extend_from_slice(arguments);
        Cli::try_parse_from(all).is_ok_and(|cli| matches!(cli.command, RootCommand::Connections(_)))
    }

    #[test]
    fn luecken_task_cli_connections_edit_and_smb_parse_like_the_gui_fields() {
        assert!(parses(&["edit", "prod", "--host", "h", "--port", "2222"]));
        assert!(parses(&["edit", "prod", "--key", "/k", "--password-stdin"]));
        assert!(parses(&["edit", "prod", "--ssh-agent"]));
        assert!(parses(&[
            "edit",
            "prod",
            "--password-auth",
            "--password",
            "x"
        ]));
        assert!(parses(&["edit", "prod", "--no-agent", "--label", "p2"]));
        assert!(!parses(&["edit", "prod", "--key", "/k", "--ssh-agent"]));
        assert!(!parses(&["edit", "prod", "--ssh-agent", "--password", "x"]));
        assert!(!parses(&["edit", "prod", "--agent", "--no-agent"]));
        assert!(!parses(&["edit"]));
        assert!(parses(&[
            "add",
            "smb",
            "--host",
            "nas",
            "--user",
            "WORK\\alice",
            "--root",
            "/public/docs",
            "--label",
            "nas",
            "--password-stdin"
        ]));
        assert!(parses(&["remove", "nas"]));
    }
}
