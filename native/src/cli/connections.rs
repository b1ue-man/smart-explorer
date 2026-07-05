use clap::{ArgAction, Args, Subcommand, ValueEnum};

use super::setup;

#[derive(Args)]
pub(super) struct ConnectionsArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    List {
        #[arg(long)]
        json: bool,
    },
    Add(ConnectionAddArgs),
    AddPeer(PeerAddArgs),
    AddRoom(RoomAddArgs),
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
    #[arg(value_enum)]
    protocol: ConnectionProtocolArg,
    #[arg(long)]
    host: Option<String>,
    #[arg(long)]
    port: Option<u16>,
    #[arg(long, default_value = "")]
    user: String,
    #[arg(long, default_value = "/")]
    root: String,
    #[arg(long, default_value = "")]
    label: String,
    #[arg(long)]
    key: Option<String>,
    #[arg(long)]
    agent: bool,
    #[arg(long, conflicts_with = "password_stdin")]
    password: Option<String>,
    #[arg(long)]
    password_stdin: bool,
}

#[derive(Args)]
struct PeerAddArgs {
    #[arg(long)]
    code: String,
    #[arg(long, default_value = "")]
    name: String,
    #[arg(long = "no-request", action = ArgAction::SetFalse, default_value_t = true)]
    request: bool,
}

#[derive(Args)]
struct RoomAddArgs {
    #[arg(long)]
    code: String,
    #[arg(long, default_value = "")]
    name: String,
}

pub(super) fn run(args: ConnectionsArgs) -> Result<i32, String> {
    match args.command {
        Command::List { json } => {
            print_connections(json)?;
            Ok(0)
        }
        Command::Add(args) => {
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
        Command::AddPeer(args) => {
            let msg = setup::add_peer(&args.code, &args.name, args.request)?;
            println!("{msg}");
            Ok(0)
        }
        Command::AddRoom(args) => {
            let msg = setup::add_room(&args.code, &args.name)?;
            println!("{msg}");
            Ok(0)
        }
    }
}

fn print_connections(json: bool) -> Result<(), String> {
    let conns = crate::creds::load_connections();
    if json {
        let rows: Vec<_> = conns
            .iter()
            .map(|c| {
                serde_json::json!({
                    "label": c.display(),
                    "account": c.account(),
                    "protocol": c.protocol.as_str(),
                    "host": c.host,
                    "port": c.port,
                    "user": c.user,
                    "root": c.root,
                    "use_agent": c.use_agent,
                })
            })
            .chain(share_connection_rows_json())
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    for c in conns {
        println!(
            "{}\t{}\t{}\t{}",
            c.display(),
            c.protocol.as_str(),
            c.to_target(),
            if c.use_agent { "agent" } else { "" }
        );
    }
    for line in share_connection_rows_text() {
        println!("{line}");
    }
    Ok(())
}

fn share_connection_rows_json() -> impl Iterator<Item = serde_json::Value> {
    let profiles = crate::share::ShareProfiles::load(None);
    let direct = profiles.direct_contacts.into_iter().map(|c| {
        let endpoint = crate::share::PeerOpenTarget::Direct {
            contact_id: c.id.clone(),
        }
        .endpoint_prefix();
        serde_json::json!({
            "label": c.display_name,
            "account": endpoint,
            "protocol": "share",
            "kind": "direct",
            "status": c.status.label(),
            "access_state": c.access_state.label(),
        })
    });
    let rooms = profiles.rooms.into_iter().flat_map(|room| {
        if room.members.is_empty() {
            return vec![serde_json::json!({
                "label": room.name,
                "account": format!("share://room/{}", room.id),
                "protocol": "share",
                "kind": "room",
                "status": room.status.label(),
            })];
        }
        room.members
            .into_iter()
            .map(|member| {
                let endpoint = crate::share::PeerOpenTarget::RoomDevice {
                    room_id: room.id.clone(),
                    device_id: member.device_id.clone(),
                }
                .endpoint_prefix();
                serde_json::json!({
                    "label": format!("{}/{}", room.name, member.device_name),
                    "account": endpoint,
                    "protocol": "share",
                    "kind": "room-member",
                    "status": member.status.label(),
                    "blocked": member.blocked,
                })
            })
            .collect::<Vec<_>>()
    });
    direct.chain(rooms)
}

fn share_connection_rows_text() -> Vec<String> {
    let profiles = crate::share::ShareProfiles::load(None);
    let mut rows = Vec::new();
    for c in profiles.direct_contacts {
        let endpoint = crate::share::PeerOpenTarget::Direct { contact_id: c.id }.endpoint_prefix();
        rows.push(format!(
            "{}\tshare\t{}\t{}",
            c.display_name,
            endpoint,
            c.access_state.label()
        ));
    }
    for room in profiles.rooms {
        if room.members.is_empty() {
            rows.push(format!(
                "{}\tshare-room\tshare://room/{}\t{}",
                room.name,
                room.id,
                room.status.label()
            ));
            continue;
        }
        for member in room.members {
            let endpoint = crate::share::PeerOpenTarget::RoomDevice {
                room_id: room.id.clone(),
                device_id: member.device_id,
            }
            .endpoint_prefix();
            rows.push(format!(
                "{}/{}\tshare\t{}\t{}",
                room.name,
                member.device_name,
                endpoint,
                member.status.label()
            ));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::super::{Cli, Command as RootCommand};

    #[test]
    fn parses_connections_list_json() {
        let cli = Cli::parse_from(["se", "connections", "list", "--json"]);
        match cli.command {
            RootCommand::Connections(args) => match args.command {
                super::Command::List { json } => assert!(json),
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
                super::Command::Add(args) => {
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
                super::Command::Add(args) => {
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
                super::Command::AddPeer(args) => {
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
                super::Command::AddPeer(args) => assert!(!args.request),
                _ => panic!("wrong command"),
            },
            _ => panic!("wrong command"),
        }
    }
}
