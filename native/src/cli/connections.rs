use clap::{ArgAction, Args, Subcommand, ValueEnum};
use clap_complete::ArgValueCandidates;

use super::setup;

#[path = "connections/output.rs"]
mod output;

const CONNECTIONS_HELP: &str = "\
Manage the same saved remotes and Smart Explorer Share contacts used by the GUI.

Examples:
  se connections list
  se connections add sftp --host example.com --user alice --root /srv --label prod --password-stdin
  se connections add webdav --host cloud.example.com --user alice --root /remote.php/dav/files/alice --label cloud --password-stdin
  se connections add share --root \"\\\\server\\share\" --label nas  (Windows only)
  se connections add-peer --code SE-D3-... --name Laptop
  se connections add-room --code SE-R3-... --name Team
  se connections remove prod
  se connections remove-peer Laptop
  se connections remove-room Team

Peer setup is one-sided: it saves to Smart Explorer's normal profile store, then
wakes the background Share worker. The other client confirms with `se share
status` plus `se share request accept`, or in the GUI. Re-running add-peer for an
existing code requeues the access request when access is still needed;
--no-request only saves it locally.
Room setup uses that same profile store and worker.";

const ADD_HELP: &str = "\
Save a configured remote connection in Smart Explorer's normal connection store.

Targets can then use @label:/path shorthand, for example:
  se ls @prod:/var/log
  se get @cloud:/Documents/report.pdf .\\report.pdf";

const ADD_PEER_HELP: &str = "\
Save a Smart Explorer direct peer from the other client's direct code.

The command is one-sided: paste the code here, and the other Smart Explorer
client receives the normal confirmation request in its CLI or GUI. Re-running the
same command with an already-saved code requeues the request when access is still
needed. Its output includes the exact selector and copyable share:// endpoint for
later file operations or removal.";

const ADD_ROOM_HELP: &str = "\
Save a Smart Explorer room from a room invite code and configure the share worker
to join it using the existing Smart Explorer share profile store.";

#[derive(Args)]
#[command(about = "Manage saved remotes and Share contacts", long_about = CONNECTIONS_HELP)]
pub(super) struct ConnectionsArgs {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "List saved GUI remotes and Share endpoints")]
    List {
        #[arg(
            long,
            help = "Print machine-readable JSON instead of tab-separated rows"
        )]
        json: bool,
    },
    #[command(
        about = "Save an SFTP/FTP/FTPS/WebDAV remote or Windows UNC share",
        long_about = ADD_HELP
    )]
    Add(ConnectionAddArgs),
    #[command(about = "Add or requeue a direct Smart Explorer peer", long_about = ADD_PEER_HELP)]
    AddPeer(PeerAddArgs),
    #[command(about = "Add a Smart Explorer share room", long_about = ADD_ROOM_HELP)]
    AddRoom(RoomAddArgs),
    #[command(about = "Remove one saved remote by exact label or account")]
    Remove(SelectorArgs),
    #[command(about = "Remove a Share peer by a listed selector; auto-selects the only peer")]
    RemovePeer(PeerSelectorArgs),
    #[command(about = "Remove one Share room by exact name or id")]
    RemoveRoom(SelectorArgs),
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ConnectionProtocolArg {
    Sftp,
    Ftp,
    Ftps,
    Webdav,
    Share,
}

impl ConnectionProtocolArg {
    fn into_protocol(self) -> crate::creds::Protocol {
        match self {
            Self::Sftp => crate::creds::Protocol::Sftp,
            Self::Ftp => crate::creds::Protocol::Ftp,
            Self::Ftps => crate::creds::Protocol::Ftps,
            Self::Webdav => crate::creds::Protocol::Webdav,
            Self::Share => crate::creds::Protocol::Share,
        }
    }
}

#[derive(Args)]
struct ConnectionAddArgs {
    #[arg(value_enum, help = "Remote kind: sftp, ftp, ftps, webdav, or share")]
    protocol: ConnectionProtocolArg,
    #[arg(
        long,
        help = "Remote host; optional for share when --root is a UNC path"
    )]
    host: Option<String>,
    #[arg(long, help = "Remote port; defaults to the protocol's standard port")]
    port: Option<u16>,
    #[arg(
        long,
        default_value = "",
        help = "Login user or DOMAIN\\user for UNC shares"
    )]
    user: String,
    #[arg(
        long,
        default_value = "/",
        help = "Remote start path, WebDAV path, or UNC path for share"
    )]
    root: String,
    #[arg(
        long,
        default_value = "",
        help = "Saved connection label used by @label:/path"
    )]
    label: String,
    #[arg(long, help = "SFTP private key path")]
    key: Option<String>,
    #[arg(
        long,
        help = "Deploy the Smart Explorer acceleration agent after SFTP login"
    )]
    agent: bool,
    #[arg(
        long,
        conflicts_with = "password_stdin",
        help = "Password or token to store in Smart Explorer's credential store"
    )]
    password: Option<String>,
    #[arg(long, help = "Read the password or token from stdin")]
    password_stdin: bool,
}

#[derive(Args)]
struct PeerAddArgs {
    #[arg(
        long,
        help = "Direct peer code copied from the other Smart Explorer client"
    )]
    code: String,
    #[arg(long, default_value = "", help = "Local display name for this peer")]
    name: String,
    #[arg(
        long,
        help = "Print selector, endpoint, request ID, and lifecycle state as JSON"
    )]
    json: bool,
    #[arg(
        long = "no-request",
        action = ArgAction::SetFalse,
        default_value_t = true,
        help = "Only save the peer locally; do not wake the share worker now"
    )]
    request: bool,
}

#[derive(Args)]
struct RoomAddArgs {
    #[arg(long, help = "Room invite code copied from Smart Explorer")]
    code: String,
    #[arg(long, default_value = "", help = "Local display name for this room")]
    name: String,
}

#[derive(Args)]
struct SelectorArgs {
    #[arg(
        allow_hyphen_values = true,
        help = "Exact saved label, account, name, or id"
    )]
    selector: String,
}

#[derive(Args)]
struct PeerSelectorArgs {
    #[arg(
        allow_hyphen_values = true,
        help = "Optional id, endpoint, name, device id, fingerprint, or prefix shown by connections list",
        add = ArgValueCandidates::new(crate::cli::completions::peer_candidates)
    )]
    selector: Option<String>,
}

pub(super) fn run(args: ConnectionsArgs) -> Result<i32, String> {
    match args.command {
        None => {
            output::print_connections(false)?;
            Ok(0)
        }
        Some(Command::List { json }) => {
            output::print_connections(json)?;
            Ok(0)
        }
        Some(Command::Add(args)) => {
            let secret = if args.password_stdin {
                Some(setup::read_stdin_secret()?)
            } else {
                args.password
            };
            let saved = setup::add_remote(setup::RemoteConnectionInput {
                protocol: args.protocol.into_protocol(),
                host: args.host,
                port: args.port,
                user: args.user,
                root: args.root,
                label: args.label,
                key: args.key,
                use_agent: args.agent,
                secret,
            })?;
            println!("{saved}");
            Ok(0)
        }
        Some(Command::AddPeer(args)) => {
            let output = setup::add_peer(&args.code, &args.name, args.request)?;
            println!("{}", output.render(args.json)?);
            Ok(0)
        }
        Some(Command::AddRoom(args)) => {
            let msg = setup::add_room(&args.code, &args.name)?;
            println!("{msg}");
            Ok(0)
        }
        Some(Command::Remove(args)) => {
            println!("{}", setup::remove_remote(&args.selector)?);
            Ok(0)
        }
        Some(Command::RemovePeer(args)) => {
            println!("{}", setup::remove_peer(args.selector.as_deref())?);
            Ok(0)
        }
        Some(Command::RemoveRoom(args)) => {
            println!("{}", setup::remove_room(&args.selector)?);
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::super::{Cli, Command as RootCommand};

    #[test]
    fn parses_connections_list_json() {
        let cli = Cli::parse_from(["se", "connections"]);
        match cli.command {
            RootCommand::Connections(args) => assert!(args.command.is_none()),
            _ => panic!("wrong command"),
        }

        let cli = Cli::parse_from(["se", "connections", "list", "--json"]);
        match cli.command {
            RootCommand::Connections(args) => match args.command {
                Some(super::Command::List { json }) => assert!(json),
                _ => panic!("wrong command"),
            },
            _ => panic!("wrong command"),
        }

        let cli = Cli::parse_from(["se", "connections", "remove-peer", "Laptop"]);
        match cli.command {
            RootCommand::Connections(args) => match args.command {
                Some(super::Command::RemovePeer(args)) => {
                    assert_eq!(args.selector.as_deref(), Some("Laptop"))
                }
                _ => panic!("wrong command"),
            },
            _ => panic!("wrong command"),
        }

        let cli = Cli::parse_from(["se", "connections", "remove-peer"]);
        match cli.command {
            RootCommand::Connections(args) => match args.command {
                Some(super::Command::RemovePeer(args)) => assert!(args.selector.is_none()),
                _ => panic!("wrong command"),
            },
            _ => panic!("wrong command"),
        }

        let cli = Cli::parse_from(["se", "connections", "remove-peer", "-peer-id"]);
        match cli.command {
            RootCommand::Connections(args) => match args.command {
                Some(super::Command::RemovePeer(args)) => {
                    assert_eq!(args.selector.as_deref(), Some("-peer-id"))
                }
                _ => panic!("wrong command"),
            },
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn parses_connection_add_with_secret_from_stdin() {
        let cli = Cli::parse_from([
            "se",
            "connections",
            "add",
            "sftp",
            "--host",
            "example.com",
            "--user",
            "alice",
            "--root",
            "/srv",
            "--label",
            "prod",
            "--password-stdin",
        ]);
        match cli.command {
            RootCommand::Connections(args) => match args.command {
                Some(super::Command::Add(args)) => {
                    assert!(matches!(args.protocol, super::ConnectionProtocolArg::Sftp));
                    assert_eq!(args.host.as_deref(), Some("example.com"));
                    assert_eq!(args.user, "alice");
                    assert_eq!(args.root, "/srv");
                    assert_eq!(args.label, "prod");
                    assert!(args.password_stdin);
                }
                _ => panic!("wrong command"),
            },
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn parses_connection_add_share_without_host() {
        let cli = Cli::parse_from([
            "se",
            "connections",
            "add",
            "share",
            "--root",
            r"\\srv\pub",
            "--label",
            "files",
        ]);
        match cli.command {
            RootCommand::Connections(args) => match args.command {
                Some(super::Command::Add(args)) => {
                    assert!(matches!(args.protocol, super::ConnectionProtocolArg::Share));
                    assert!(args.host.is_none());
                    assert_eq!(args.root, r"\\srv\pub");
                }
                _ => panic!("wrong command"),
            },
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn parses_peer_setup_with_default_request_and_opt_out() {
        let cli = Cli::parse_from([
            "se",
            "connections",
            "add-peer",
            "--code",
            "SE-D3-a-0000000000000000000000000000000000000000000000000000000000000000-11111111111111111111111111111111-node",
            "--name",
            "Laptop",
        ]);
        match cli.command {
            RootCommand::Connections(args) => match args.command {
                Some(super::Command::AddPeer(args)) => {
                    assert_eq!(args.name, "Laptop");
                    assert!(args.request);
                }
                _ => panic!("wrong command"),
            },
            _ => panic!("wrong command"),
        }

        let cli = Cli::parse_from([
            "se",
            "connections",
            "add-peer",
            "--code",
            "SE-D3-a-0000000000000000000000000000000000000000000000000000000000000000-11111111111111111111111111111111-node",
            "--no-request",
        ]);
        match cli.command {
            RootCommand::Connections(args) => match args.command {
                Some(super::Command::AddPeer(args)) => assert!(!args.request),
                _ => panic!("wrong command"),
            },
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn connections_help_explains_one_sided_peer_setup() {
        let mut command = Cli::command();
        let help = command
            .find_subcommand_mut("connections")
            .unwrap()
            .render_long_help()
            .to_string();
        assert!(help.contains("one-sided"));
        assert!(help.contains("requeues the access request"));
    }
}
