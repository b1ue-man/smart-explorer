//! `se shell`: a remote shell in the terminal. In a terminal (standard input
//! and output are terminals, no `-T`) it is a terminal session like `ssh HOST`
//! (docs/plaene/2026-10-14-echtes-terminal/plan.md): the remote runs its login
//! shell in a pseudo-terminal and the local terminal is in raw mode
//! (`shell_terminal.rs`, escape sequences in `shell_escape.rs`). Otherwise, with
//! `-T`, or when the remote cannot give a pseudo-terminal, it is the
//! line-based command shell (docs/plaene/2026-10-14-analyse-id-plugin-vorrang/
//! plan.md, R4): every line is a command that keeps the working directory,
//! environment and shell state of the earlier ones (`shell_repl.rs`).
//! With `--attachable` the user's other `se` processes may join the session
//! (another terminal; an AI agent's `sessions` tool joins command sessions);
//! `--attach` joins such a session or an AI agent's session.
#[path = "shell_escape.rs"]
mod escape;
#[path = "shell_repl.rs"]
mod repl;
#[path = "shell_terminal.rs"]
mod terminal;

use super::shell_session::{
    self, Dialect, OpenOptions, SessionClient, SessionKind, TerminalClient, TerminalOpen,
    TerminalOptions,
};
use clap::Args;
use clap_complete::ArgValueCandidates;
use escape::{parse_escape_char, EscapeChar};
use std::io::IsTerminal;

/// Exit code of a session that could not be joined or run (as in the sessions).
const EXIT_FAILED: i32 = 125;

const SHELL_HELP: &str = "\
Open a remote shell on a saved SFTP remote (@label[:/dir], its sftp:// endpoint)
or on a Share peer that granted this device Exec (name, id, share:// endpoint;
omit it when exactly one peer is ready).

In a terminal this works like `ssh HOST`: the remote gets a pseudo-terminal and
runs its login shell in it, and your terminal passes every key and all output
through unchanged. Full-screen programs (vim, nvim, claude, codex), colours,
Tab completion, password prompts and the window size work as over ssh; Ctrl+C
and Ctrl+D go to the program on the remote. The session ends when the remote
shell exits. The terminal is restored on every way out. Exit code: the remote
shell's exit status; 125 when the session or the connection failed or an update
ends this shell; 130 after ~. (below); 128 plus the signal number after a local
SIGTERM, SIGHUP, SIGQUIT or SIGINT (Windows: 130 after Ctrl+Break, closing the
window, logoff or shutdown).

Escape sequences, only at the start of a line, as in ssh(1):
  ~.  leave: the owner ends the session (exit code 130), a joined terminal
      detaches (exit code 0)
  ~~  send one ~
  ~?  list the sequences
Any other byte after ~ is sent together with the ~. --escape-char CHAR sets
another ASCII character (not . or ?); --escape-char none turns them off.

The line-based command shell runs instead when standard input or output is not
a terminal, with -T/--no-tty (like ssh -T), or when the remote cannot give a
pseudo-terminal (se says why on standard error). There every line is one
command; cd, export, variables and functions persist like in an SSH session. A
line ending in \\ (PowerShell: `) continues on the next line. The prompt shows
target and working directory on standard error. Without a terminal, every input
line is one command and gets no input of its own. There is no pseudo-terminal in
this mode: full-screen programs and password prompts do not work. Ctrl+C ends
the whole session (there is no signal path to one command); Ctrl+D or the end of
input closes it. The exit code is that of the last command, of `exit N`, 130
after Ctrl+C, or 125 when the session failed.

--attachable lets the user's other se processes join this session. Only
processes of the same OS user can join, and only while this shell runs. Without
it nobody can join. Another terminal joins a terminal session with
`se shell --attach NAME` and shares its one remote terminal with the owner: both
see the screen and both type into it, and the remote terminal takes the size of
the terminal used last; ~. detaches it. A terminal session can only be joined
from a terminal and not with -T. AI agents (the plugin's `sessions` tool) join
only command sessions: to let an agent join your shell, start it with
`se shell -T --attachable`.

--attach joins a running attachable session by name or id. A command session (an
AI agent's, attachable unless the agent opened it with attachable=false, or one
started with `se shell -T --attachable`) is joined in the line-based mode:
commands run in that shared shell one after another; Ctrl+D detaches and the
session keeps running; Ctrl+C or `exit` ends it for everyone.

--list prints the attachable sessions: name, id, shell dialect (- if unknown),
target, owner and kind (terminal or commands).

Examples:
  se shell @prod:/srv/app
  se shell Laptop --attachable --name build
  se shell -T @prod --attachable
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
    #[arg(
        short = 'T',
        long = "no-tty",
        help = "Run the line-based command shell even in a terminal (like ssh -T)"
    )]
    pub(super) no_tty: bool,
    #[arg(
        short = 'e',
        long = "escape-char",
        value_name = "CHAR|none",
        default_value = "~",
        value_parser = parse_escape_char,
        help = "Escape character of a terminal session: one ASCII character, or none to turn the sequences off"
    )]
    pub(super) escape_char: EscapeChar,
}

pub(super) fn run(args: ShellArgs) -> Result<i32, String> {
    if args.list {
        return list(args.json);
    }
    let label = format!("se shell (pid {})", std::process::id());
    let in_terminal =
        !args.no_tty && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    match &args.attach {
        Some(selector) => attach(selector, label, in_terminal, &args),
        None => open(&args, label, in_terminal),
    }
}

/// Joins a running session: a terminal session from a terminal, a command
/// session in the line-based mode (also in a terminal, and with `-T`).
fn attach(
    selector: &str,
    label: String,
    in_terminal: bool,
    args: &ShellArgs,
) -> Result<i32, String> {
    let entry = shell_session::find_attachable(selector)?;
    if entry.kind == SessionKind::Terminal {
        if !in_terminal {
            return refuse(&format!(
                "session {} is a terminal session; join it from a terminal without -T",
                entry.name
            ));
        }
        if let Err(why) = terminal::raw_mode_available() {
            return refuse(&format!(
                "this terminal cannot run a full terminal session ({why}); it cannot join session {}",
                entry.name
            ));
        }
        let size = terminal::local_size();
        let client = TerminalClient::attach(&entry, &label, size)?;
        eprintln!(
            "se: attached to session {} ({}) on {}, owned by {}; {}",
            entry.name,
            entry.id,
            entry.target,
            entry.owner,
            detach_hint(args.escape_char.0)
        );
        return Ok(terminal::run(client, args.escape_char.0, size));
    }
    let client = SessionClient::attach(&entry, &label)?;
    eprintln!(
        "se: attached to session {} ({}) on {}, owned by {}; Ctrl+D detaches, Ctrl+C ends the session for everyone",
        entry.name, entry.id, entry.target, entry.owner
    );
    Ok(repl::Repl::new(client, std::io::stdin().is_terminal()).run())
}

/// Opens a session on the target: a terminal session in a terminal, falling
/// back to the command shell when the terminal or the remote cannot do it.
fn open(args: &ShellArgs, label: String, mut in_terminal: bool) -> Result<i32, String> {
    let target = shell_session::resolve_target(args.target.as_deref())?;
    let name = args
        .name
        .clone()
        .unwrap_or_else(|| format!("shell-{}", std::process::id()));
    let env = super::exec::parse_environment(args.env.clone())?;
    if in_terminal {
        if let Err(why) = terminal::raw_mode_available() {
            eprintln!(
                "se: this terminal cannot run a full terminal session ({why}); using the command shell"
            );
            in_terminal = false;
        }
    }
    if in_terminal {
        let size = terminal::local_size();
        let options = TerminalOptions {
            name: name.clone(),
            owner: label.clone(),
            attachable: args.attachable,
            cwd: args.cwd.clone(),
            env: env.clone(),
            term: terminal::local_term(),
            size,
        };
        match shell_session::open_terminal(
            &target,
            &super::host_key_prompt::open_saved_ssh,
            options,
        )? {
            TerminalOpen::Ready(client) => {
                if args.attachable {
                    let info = client.info();
                    eprintln!("{}", joinable_hint(&info.name, &info.id));
                }
                return Ok(terminal::run(client, args.escape_char.0, size));
            }
            TerminalOpen::Unsupported(reason) => {
                eprintln!("se: {reason}; using the command shell");
            }
        }
    }
    let options = OpenOptions {
        name,
        owner: label,
        attachable: args.attachable,
        cwd: args.cwd.clone(),
        env,
    };
    let client = shell_session::open(&target, &super::host_key_prompt::open_saved_ssh, options)?;
    if args.attachable {
        let info = client.info();
        eprintln!("{}", joinable_hint(&info.name, &info.id));
    }
    Ok(repl::Repl::new(client, std::io::stdin().is_terminal()).run())
}

/// Prints `message` as an error and ends with the session failure code.
fn refuse(message: &str) -> Result<i32, String> {
    eprintln!("se: {message}");
    Ok(EXIT_FAILED)
}

fn joinable_hint(name: &str, id: &str) -> String {
    format!("se: session {name} ({id}) can be joined with: se shell --attach {name}")
}

/// How a joined terminal leaves again, for the line printed on joining.
fn detach_hint(escape: Option<u8>) -> String {
    match escape {
        Some(escape) => format!("{}. detaches", char::from(escape)),
        None => "the escape sequences are off".to_string(),
    }
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
            "{}\t{}\t{}\t{}\t{}\t{}",
            entry.name,
            entry.id,
            entry.dialect.map_or("-", Dialect::label),
            entry.target,
            entry.owner,
            entry.kind.label()
        );
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::{detach_hint, joinable_hint};

    #[test]
    fn terminal_task_hints_name_the_session_and_the_leave_sequence() {
        assert_eq!(
            joinable_hint("build", "0123abcd"),
            "se: session build (0123abcd) can be joined with: se shell --attach build"
        );
        assert_eq!(detach_hint(Some(b'~')), "~. detaches");
        assert_eq!(detach_hint(Some(b'#')), "#. detaches");
        assert_eq!(detach_hint(None), "the escape sequences are off");
    }
}
