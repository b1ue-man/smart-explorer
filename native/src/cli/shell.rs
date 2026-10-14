//! `se shell`: a persistent remote shell in the terminal
//! (docs/plaene/2026-10-14-analyse-id-plugin-vorrang/plan.md, R4 S3 and the
//! attach extension). Without `--attach` it opens a session on a saved SFTP
//! remote or a Share peer; every line is a command that keeps the working
//! directory, environment and shell state of the earlier ones. With
//! `--attachable` the user's other `se` processes (another terminal, an AI
//! agent's `sessions` tool) may join it; `--attach` joins such a session or an
//! AI agent's session. Other participants' commands and output are shown as
//! they happen.
#[path = "shell_repl.rs"]
mod repl;

use super::shell_session::{self, OpenOptions, SessionClient};
use clap::Args;
use clap_complete::ArgValueCandidates;
use std::io::IsTerminal;

const SHELL_HELP: &str = "\
Open a persistent remote shell on a saved SFTP remote (@label[:/dir], its
sftp:// endpoint) or on a Share peer that granted this device Exec (name, id,
share:// endpoint; omit it when exactly one peer is ready). Every line is one
command; cd, export, variables and functions persist like in an SSH session.
A line ending in \\ (PowerShell: `) continues on the next line. The prompt
shows target and working directory on standard error. Without a terminal,
every input line is one command and gets no input of its own.

There is no PTY: full-screen programs and password prompts do not work.
Ctrl+C ends the whole session (there is no signal path to one command);
Ctrl+D or the end of input closes it. The exit code is that of the last
command, of `exit N`, 130 after Ctrl+C, or 125 when the session failed.

--attachable lets the user's other se processes join this session: another
`se shell --attach NAME`, or an AI agent through the plugin's `sessions` tool.
Only processes of the same OS user can join, and only while this shell runs.
Without it nobody can join.

--attach joins a running attachable session by name or id, for example an AI
agent's session (they are attachable unless the agent opened them with
attachable=false). Commands then run in that shared shell one after another;
Ctrl+D detaches and the session keeps running; Ctrl+C or `exit` ends it for
everyone.

Examples:
  se shell @prod:/srv/app
  se shell Laptop --attachable --name build
  se shell --list
  se shell --attach build";

#[derive(Args)]
#[command(long_about = SHELL_HELP)]
pub(super) struct ShellArgs {
    #[arg(
        help = "Peer selector, share:// endpoint, or saved SFTP remote (@label[:/dir]); omit when exactly one peer is ready",
        add = ArgValueCandidates::new(crate::cli::completions::exec_target_candidates)
    )]
    pub(super) target: Option<String>,
    #[arg(long, help = "Remote working directory to start in")]
    pub(super) cwd: Option<String>,
    #[arg(
        long = "env",
        value_name = "NAME=VALUE",
        help = "Set one remote environment value at the start; repeatable"
    )]
    pub(super) env: Vec<String>,
    #[arg(long, help = "Session name others attach with (default shell-<pid>)")]
    pub(super) name: Option<String>,
    #[arg(
        long,
        help = "Let the user's other se processes (se shell --attach, AI agents) join this session"
    )]
    pub(super) attachable: bool,
    #[arg(
        long,
        value_name = "SESSION",
        conflicts_with_all = ["target", "cwd", "env", "name", "attachable", "list"],
        help = "Join a running attachable session by name or id"
    )]
    pub(super) attach: Option<String>,
    #[arg(
        long,
        conflicts_with_all = ["target", "cwd", "env", "name", "attachable"],
        help = "List the attachable sessions of this user"
    )]
    pub(super) list: bool,
    #[arg(long, requires = "list", help = "With --list: print JSON")]
    pub(super) json: bool,
}

pub(super) fn run(args: ShellArgs) -> Result<i32, String> {
    if args.list {
        return list(args.json);
    }
    let label = format!("se shell (pid {})", std::process::id());
    let client = match &args.attach {
        Some(selector) => {
            let entry = shell_session::find_attachable(selector)?;
            let client = SessionClient::attach(&entry, &label)?;
            eprintln!(
                "se: attached to session {} ({}) on {}, owned by {}; Ctrl+D detaches, Ctrl+C ends the session for everyone",
                entry.name, entry.id, entry.target, entry.owner
            );
            client
        }
        None => {
            let target = shell_session::resolve_target(args.target.as_deref())?;
            let options = OpenOptions {
                name: args
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("shell-{}", std::process::id())),
                owner: label,
                attachable: args.attachable,
                cwd: args.cwd.clone(),
                env: super::exec::parse_environment(args.env.clone())?,
            };
            let client = shell_session::open(&target, &crate::connect::open_saved_ssh, options)?;
            if args.attachable {
                let info = client.info();
                eprintln!(
                    "se: session {} ({}) can be joined with: se shell --attach {}",
                    info.name, info.id, info.name
                );
            }
            client
        }
    };
    Ok(repl::Repl::new(client, std::io::stdin().is_terminal()).run())
}

fn list(json: bool) -> Result<i32, String> {
    let entries = shell_session::list_attachable();
    if json {
        let values = entries
            .iter()
            .map(|entry| entry.public_value())
            .collect::<Vec<_>>();
        let text = serde_json::to_string_pretty(&values).map_err(|error| error.to_string())?;
        println!("{text}");
        return Ok(0);
    }
    if entries.is_empty() {
        println!("no attachable sessions; start one with `se shell TARGET --attachable`");
    }
    for entry in &entries {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            entry.name,
            entry.id,
            entry.dialect.label(),
            entry.target,
            entry.owner
        );
    }
    Ok(0)
}
