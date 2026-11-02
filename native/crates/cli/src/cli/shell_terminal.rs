//! The terminal front-end of `se shell`, like `ssh HOST` in a terminal: the
//! local terminal is in raw mode, every typed byte goes to the remote terminal
//! and every byte of the remote terminal goes to the screen unchanged, the
//! remote window follows the local one, and the escape sequences `~.`, `~~` and
//! `~?` work as in `ssh` (`shell_escape.rs`). The owner of a session and every
//! terminal that joined it with `se shell --attach` run the same loop.
//!
//! The terminal is restored on every way out before anything is written to
//! standard error: when the session ends, on `~.`, on a failure, on an update
//! stop request and on a local request to end (Unix signals, Windows console
//! control events). Platform code lives in `os/terminal_{unix,windows}.rs`.
#[path = "shell_output_filter.rs"]
mod output_filter;

use super::escape::{help_text, EscapeFilter, Step};
use crate::cli::os::{self, LocalEvent, LocalTerminal, TerminalOutput};
use crate::cli::shell_session::{Role, TerminalClient, TerminalEvent, TerminalSize};
use crate::share::ExecTerminalRequest;
use crossbeam_channel::{select, unbounded, Receiver, Sender};
use output_filter::OutputFilter;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Exit code when the session or the connection failed (as in the line mode).
const EXIT_FAILED: i32 = 125;
/// Exit code after the owner left with the escape sequence.
const EXIT_OWNER_LEFT: i32 = 130;
/// Exit code after an attached terminal detached.
const EXIT_DETACHED: i32 = 0;
/// How long a leaving owner waits for the session to wind down.
const OWNER_WIND_DOWN: Duration = Duration::from_secs(10);
/// Largest columns and rows value of a terminal (the limit of `TerminalSize`).
const MAX_DIMENSION: u16 = 32_767;
/// Size assumed when the terminal does not report one.
const DEFAULT_SIZE: TerminalSize = TerminalSize { cols: 80, rows: 24 };
/// `TERM` for the remote when the local one is unset or not acceptable.
const DEFAULT_TERM: &str = "xterm-256color";

/// Whether this terminal can run a full terminal session (checks raw mode and
/// leaves the terminal as it was). The error tells why not.
pub(super) fn raw_mode_available() -> Result<(), String> {
    os::check_raw_mode()
}

/// The size of the local terminal window, 80 by 24 if it reports none.
pub(super) fn local_size() -> TerminalSize {
    os::window_size().map_or(DEFAULT_SIZE, |(cols, rows)| clamp_size(cols, rows))
}

/// The `TERM` the remote terminal gets: the local one when it is acceptable
/// to the remote (1 to 64 characters of `A-Za-z0-9._+-`), else `xterm-256color`.
pub(super) fn local_term() -> String {
    accepted_term(std::env::var("TERM").ok())
}

fn accepted_term(value: Option<String>) -> String {
    value
        .filter(|term| {
            ExecTerminalRequest {
                term: term.clone(),
                size: DEFAULT_SIZE.into(),
            }
            .validate()
            .is_ok()
        })
        .unwrap_or_else(|| DEFAULT_TERM.to_string())
}

fn clamp_size(cols: u16, rows: u16) -> TerminalSize {
    TerminalSize {
        cols: cols.clamp(1, MAX_DIMENSION),
        rows: rows.clamp(1, MAX_DIMENSION),
    }
}

/// How a terminal session ended for this front-end.
#[derive(Debug, PartialEq, Eq)]
enum End {
    /// The session ended by itself: the remote shell exited, the owner closed
    /// it or the connection was lost.
    Session {
        exit_code: Option<i32>,
        message: String,
    },
    /// The user typed the leave sequence.
    Left,
    /// The process was asked to end; the exit code comes from the platform.
    Terminated(i32),
    /// Smart Explorer is being updated.
    Update,
    /// The terminal or the connection failed.
    Failed(String),
}

impl End {
    fn exit_code(&self, role: Role) -> i32 {
        match self {
            Self::Session { exit_code, .. } => exit_code.unwrap_or(EXIT_FAILED),
            Self::Left => match role {
                Role::Owner => EXIT_OWNER_LEFT,
                Role::Attached => EXIT_DETACHED,
            },
            Self::Terminated(code) => *code,
            Self::Update | Self::Failed(_) => EXIT_FAILED,
        }
    }

    fn message(&self, role: Role) -> String {
        match self {
            Self::Session { message, .. } => format!("se: session ended: {message}"),
            Self::Left => match role {
                Role::Owner => "se: session closed".to_string(),
                Role::Attached => "se: detached from the session".to_string(),
            },
            Self::Terminated(_) => {
                "se: ended by a request of the system; the session was left".to_string()
            }
            Self::Update => "se: Smart Explorer is being updated; this shell ends now.".to_string(),
            Self::Failed(text) => format!("se: {text}"),
        }
    }
}

/// The end to report. A session that ended without an exit status, or a
/// failure, after a new update stop request is the update stop: the update
/// also stops the Share worker that carries a session to a Share peer, and
/// that end can come before the watcher's next look (plan V10). An exit
/// status of the remote shell and the user's own endings stay.
fn settle(end: End, updated: bool) -> End {
    let lost = match &end {
        End::Session { exit_code, .. } => exit_code.is_none(),
        End::Failed(_) => true,
        End::Left | End::Terminated(_) | End::Update => false,
    };
    if updated && lost {
        End::Update
    } else {
        end
    }
}

/// The rule of the line mode and the MCP server for a desktop update's stop
/// request: one present at the start does not count, a new or rewritten one
/// does.
fn new_update_request(baseline: Option<SystemTime>, current: Option<SystemTime>) -> bool {
    current.is_some() && current != baseline
}

/// What goes to the remote terminal, in order, from its own thread so that a
/// large paste cannot keep this loop from draining the remote output.
enum Outgoing {
    Bytes(Vec<u8>),
    Size(TerminalSize),
}

/// Runs the terminal session until it or the user ends it and returns the exit
/// code: the remote shell's exit status; 130 after the owner's `~.`; 0 after an
/// attached terminal's `~.`; 125 when something failed or the update stops this
/// shell; the platform's code after a local request to end.
pub(super) fn run(client: TerminalClient, escape: Option<u8>, size: TerminalSize) -> i32 {
    let role = client.info().role;
    let (local_sender, local_events) = unbounded::<LocalEvent>();
    let terminal = match LocalTerminal::enter(local_sender.clone()) {
        Ok(terminal) => terminal,
        Err(error) => {
            client.leave();
            say(&format!("se: {error}"));
            return EXIT_FAILED;
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let (update_sender, update_events) = unbounded::<()>();
    let baseline = crate::daemon::update_stop_marker();
    let prepared = watch_updates(baseline, stop.clone(), update_sender.clone())
        .and_then(|()| spawn_outbox(&client));
    let end = match prepared {
        Ok(outbox) => {
            let mut front = Front {
                client: client.clone(),
                outbox,
                filter: EscapeFilter::new(escape),
                remote: OutputFilter::default(),
                output: TerminalOutput::default(),
                escape,
                size,
            };
            front.pump(&local_events, &client.events(), &update_events)
        }
        Err(error) => End::Failed(error),
    };
    stop.store(true, Ordering::SeqCst);
    client.leave();
    drop(terminal);
    let waited = role == Role::Owner && !matches!(end, End::Session { .. });
    if waited {
        client.wait_ended(OWNER_WIND_DOWN);
    }
    let updated = new_update_request(baseline, crate::daemon::update_stop_marker());
    let end = settle(end, updated);
    say(&end.message(role));
    end.exit_code(role)
}

struct Front {
    client: TerminalClient,
    outbox: Sender<Outgoing>,
    filter: EscapeFilter,
    /// Keeps the remote's win32-input-mode request from the local terminal.
    remote: OutputFilter,
    output: TerminalOutput,
    escape: Option<u8>,
    /// The size last sent to the session.
    size: TerminalSize,
}

impl Front {
    fn pump(
        &mut self,
        local: &Receiver<LocalEvent>,
        session: &Receiver<TerminalEvent>,
        update: &Receiver<()>,
    ) -> End {
        // The window may have changed between reading its size and the raw mode.
        if let Some((cols, rows)) = os::window_size() {
            self.resize(cols, rows);
        }
        loop {
            select! {
                recv(local) -> event => match event {
                    Ok(event) => if let Some(end) = self.local(event) {
                        return end;
                    },
                    Err(_) => return End::Failed("the local terminal stopped reporting".into()),
                },
                recv(session) -> event => match event {
                    Ok(event) => if let Some(end) = self.session(event) {
                        return end;
                    },
                    Err(_) => return self.lost(),
                },
                recv(update) -> _ => return End::Update,
            }
        }
    }

    fn local(&mut self, event: LocalEvent) -> Option<End> {
        match event {
            LocalEvent::Input(bytes) => self.typed(&bytes),
            LocalEvent::Resized(cols, rows) => {
                self.resize(cols, rows);
                None
            }
            LocalEvent::InputClosed => Some(End::Failed("the local terminal was closed".into())),
            LocalEvent::Terminate(code) => Some(End::Terminated(code)),
        }
    }

    /// Typed bytes through the escape sequences to the remote terminal.
    fn typed(&mut self, bytes: &[u8]) -> Option<End> {
        for step in self.filter.feed(bytes) {
            match step {
                Step::Send(data) => {
                    // A failed send means the connection is gone; the session's
                    // own end event follows and carries the reason.
                    let _ = self.outbox.send(Outgoing::Bytes(data));
                }
                Step::Help => {
                    let help = self.escape.map(help_text).unwrap_or_default();
                    if let Some(end) = self.show(help.as_bytes()) {
                        return Some(end);
                    }
                }
                Step::Leave => return Some(End::Left),
            }
        }
        None
    }

    /// Only a real change of the window size is passed on.
    fn resize(&mut self, cols: u16, rows: u16) {
        let size = clamp_size(cols, rows);
        if size != self.size {
            self.size = size;
            let _ = self.outbox.send(Outgoing::Size(size));
        }
    }

    fn session(&mut self, event: TerminalEvent) -> Option<End> {
        match event {
            TerminalEvent::Output(bytes) => {
                let shown = self.remote.filter(&bytes);
                if shown.is_empty() {
                    None
                } else {
                    self.show(&shown)
                }
            }
            TerminalEvent::Notice(text) => {
                self.show(format!("\r\n[se] {}\r\n", plain(&text)).as_bytes())
            }
            TerminalEvent::Ended { exit_code, message } => {
                Some(End::Session { exit_code, message })
            }
        }
    }

    /// The session's events stopped without an end event.
    fn lost(&self) -> End {
        match self.client.ended() {
            Some((exit_code, message)) => End::Session { exit_code, message },
            None => End::Failed("the connection to the session was lost".into()),
        }
    }

    fn show(&mut self, bytes: &[u8]) -> Option<End> {
        self.output
            .write(bytes)
            .err()
            .map(|error| End::Failed(format!("cannot write to the terminal: {error}")))
    }
}

/// `text` without control characters, so that it cannot steer the terminal.
fn plain(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_control()).collect()
}

fn say(text: &str) {
    let _ = writeln!(std::io::stderr(), "{text}");
}

fn spawn_outbox(client: &TerminalClient) -> Result<Sender<Outgoing>, String> {
    let client = client.clone();
    let (sender, receiver) = unbounded::<Outgoing>();
    std::thread::Builder::new()
        .name("se-term-send".into())
        .spawn(move || {
            for item in receiver {
                let sent = match item {
                    Outgoing::Bytes(bytes) => client.input(&bytes),
                    Outgoing::Size(size) => client.resize(size),
                };
                if sent.is_err() {
                    return;
                }
            }
        })
        .map_err(|error| format!("input sender: {error}"))?;
    Ok(sender)
}

/// Reports a desktop-update stop request that appears while the shell runs
/// (`baseline` is the marker at the start).
fn watch_updates(
    baseline: Option<SystemTime>,
    stop: Arc<AtomicBool>,
    notify: Sender<()>,
) -> Result<(), String> {
    std::thread::Builder::new()
        .name("se-shell-update".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            if stop.load(Ordering::SeqCst) {
                return;
            }
            if new_update_request(baseline, crate::daemon::update_stop_marker()) {
                let _ = notify.send(());
                return;
            }
        })
        .map(|_| ())
        .map_err(|error| format!("update watcher: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{
        accepted_term, clamp_size, new_update_request, plain, settle, End, DEFAULT_SIZE,
        DEFAULT_TERM,
    };
    use crate::cli::shell_session::Role;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn terminal_task_exit_codes_follow_the_ending() {
        let shell = |code| End::Session {
            exit_code: code,
            message: "the shell exited".into(),
        };
        assert_eq!(shell(Some(7)).exit_code(Role::Owner), 7);
        assert_eq!(shell(Some(0)).exit_code(Role::Attached), 0);
        assert_eq!(shell(Some(130)).exit_code(Role::Owner), 130);
        // The session ended without an exit status (connection lost, cancelled).
        assert_eq!(shell(None).exit_code(Role::Owner), 125);
        assert_eq!(shell(None).exit_code(Role::Attached), 125);
        assert_eq!(End::Left.exit_code(Role::Owner), 130);
        assert_eq!(End::Left.exit_code(Role::Attached), 0);
        assert_eq!(End::Terminated(143).exit_code(Role::Owner), 143);
        assert_eq!(End::Update.exit_code(Role::Owner), 125);
        assert_eq!(End::Failed("x".into()).exit_code(Role::Attached), 125);
    }

    #[test]
    fn terminal_task_closing_messages_name_the_reason() {
        let ended = End::Session {
            exit_code: Some(3),
            message: "the shell exited with status 3".into(),
        };
        assert_eq!(
            ended.message(Role::Owner),
            "se: session ended: the shell exited with status 3"
        );
        assert_eq!(
            End::Update.message(Role::Owner),
            "se: Smart Explorer is being updated; this shell ends now."
        );
        assert_eq!(End::Left.message(Role::Owner), "se: session closed");
        assert_eq!(
            End::Left.message(Role::Attached),
            "se: detached from the session"
        );
        assert_eq!(
            End::Failed("the local terminal was closed".into()).message(Role::Owner),
            "se: the local terminal was closed"
        );
    }

    #[test]
    fn terminal_task_the_remote_term_is_the_local_one_only_when_acceptable() {
        for good in ["xterm-256color", "vt100", "linux", "screen.xterm-256color"] {
            assert_eq!(accepted_term(Some(good.into())), good);
        }
        let too_long = "a".repeat(65);
        for bad in ["", "x y", "x;y", "é", too_long.as_str()] {
            assert_eq!(accepted_term(Some(bad.into())), DEFAULT_TERM, "{bad:?}");
        }
        assert_eq!(accepted_term(None), DEFAULT_TERM);
    }

    #[test]
    fn terminal_task_window_sizes_and_notices_are_kept_in_bounds() {
        assert_eq!((DEFAULT_SIZE.cols, DEFAULT_SIZE.rows), (80, 24));
        let huge = clamp_size(u16::MAX, u16::MAX);
        assert_eq!((huge.cols, huge.rows), (32_767, 32_767));
        let tiny = clamp_size(0, 0);
        assert_eq!((tiny.cols, tiny.rows), (1, 1));
        let normal = clamp_size(120, 40);
        assert_eq!((normal.cols, normal.rows), (120, 40));
        assert_eq!(plain("a\u{1b}[31mb\r\nc\u{7}"), "a[31mbc");
    }

    #[test]
    fn terminal_task_a_lost_session_after_an_update_request_is_the_update_stop() {
        let lost = || End::Session {
            exit_code: None,
            message: "remote execution transport: failed to fill whole buffer".into(),
        };
        let exited = || End::Session {
            exit_code: Some(7),
            message: "the shell exited with status 7".into(),
        };
        let failed = End::Failed("the connection to the session was lost".into());
        assert_eq!(settle(lost(), true), End::Update);
        assert_eq!(settle(failed, true), End::Update);
        assert_eq!(settle(lost(), false), lost());
        assert_eq!(settle(exited(), true), exited());
        assert_eq!(settle(End::Left, true), End::Left);
        assert_eq!(settle(End::Terminated(143), true), End::Terminated(143));
        let old = UNIX_EPOCH + Duration::from_secs(10);
        let new = UNIX_EPOCH + Duration::from_secs(20);
        assert!(!new_update_request(None, None));
        assert!(new_update_request(None, Some(new)));
        assert!(!new_update_request(Some(old), Some(old)));
        assert!(new_update_request(Some(old), Some(new)));
        assert!(!new_update_request(Some(old), None));
    }
}
