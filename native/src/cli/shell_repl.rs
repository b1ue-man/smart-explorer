//! The interactive loop of `se shell`: input lines (with continuation),
//! Ctrl+C and update stop requests as one event stream, the own command's
//! output and end, and the other participants' activity.
use crate::cli::shell_session::{
    Activity, CommandOutcome, CommandRequest, CommandSink, Dialect, OutcomeStatus, Role,
    SessionClient, StdinMode, Stream,
};
use crossbeam_channel::{select, unbounded, Sender};
use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

enum Input {
    Line(String),
    Eof,
    Interrupt,
    UpdateStop,
}

pub(super) struct Repl {
    client: SessionClient,
    interactive: bool,
    dialect: Dialect,
}

impl Repl {
    pub(super) fn new(client: SessionClient, interactive: bool) -> Self {
        let dialect = client.info().dialect;
        Self {
            client,
            interactive,
            dialect,
        }
    }

    pub(super) fn run(self) -> i32 {
        let (inputs, input_events) = unbounded();
        if let Err(error) = start_inputs(inputs) {
            eprintln!("se: {error}");
            return self.finish(125);
        }
        let activity = self.client.activity();
        let (done, outcomes) = unbounded::<CommandOutcome>();
        let mut lines = VecDeque::new();
        let mut partial = None;
        let mut running = false;
        let mut eof = false;
        let mut interrupted = false;
        let mut last_exit = 0;
        self.prompt(false);
        loop {
            select! {
                recv(input_events) -> input => match input.unwrap_or(Input::Eof) {
                    Input::Line(line) if running => {
                        if self.interactive && self.dialect == Dialect::Posix {
                            let _ = self.client.input(format!("{line}\n").as_bytes());
                        } else {
                            lines.push_back(line);
                        }
                    }
                    Input::Line(line) => {
                        if let Some(command) = complete(self.dialect, &mut partial, line) {
                            running = self.submit(command, &done);
                        }
                        if !running {
                            self.prompt(partial.is_some());
                        }
                    }
                    Input::Eof => {
                        eof = true;
                        if !running && lines.is_empty() {
                            return self.finish(last_exit);
                        }
                    }
                    Input::Interrupt => {
                        interrupted = true;
                        self.client.cancel();
                    }
                    Input::UpdateStop => {
                        eprintln!("se: Smart Explorer is being updated; this shell ends now.");
                        return self.finish(125);
                    }
                },
                recv(outcomes) -> outcome => if let Ok(outcome) = outcome {
                    running = false;
                    last_exit = exit_code(&outcome);
                    if outcome.session_ended {
                        if let Some(message) = &outcome.message {
                            eprintln!("se: {message}");
                        }
                        return if interrupted { 130 } else { last_exit };
                    }
                    if let Some(message) = outcome.message.as_deref().filter(|_| outcome.status != OutcomeStatus::Completed) {
                        eprintln!("se: {message}");
                    }
                    if self.interactive && last_exit != 0 {
                        eprintln!("[exit {last_exit}]");
                    }
                    while !running {
                        let Some(line) = lines.pop_front() else { break };
                        if let Some(command) = complete(self.dialect, &mut partial, line) {
                            running = self.submit(command, &done);
                        }
                    }
                    if !running {
                        if eof {
                            return self.finish(last_exit);
                        }
                        self.prompt(partial.is_some());
                    }
                },
                recv(activity) -> event => match event {
                    Ok(event) => {
                        if let Some(code) = self.show(event, running, interrupted, last_exit) {
                            return code;
                        }
                    }
                    // The client keeps the sender while it lives.
                    Err(_) => {}
                },
                default(Duration::from_secs(1)) => {
                    if !running {
                        if let Some((code, message)) = self.client.ended() {
                            eprintln!("se: session ended: {message}");
                            return if interrupted { 130 } else { code.unwrap_or(last_exit) };
                        }
                    }
                }
            }
        }
    }

    fn submit(&self, command: String, done: &Sender<CommandOutcome>) -> bool {
        let stdin = if self.interactive && self.dialect == Dialect::Posix {
            StdinMode::Interactive
        } else {
            StdinMode::Null
        };
        let request = CommandRequest {
            command,
            cwd: None,
            env: BTreeMap::new(),
            stdin,
        };
        let printer = Arc::new(Printer { done: done.clone() });
        match self.client.submit(request, printer) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("se: {error}");
                false
            }
        }
    }

    /// Others' activity: command lines and notices on stderr; their output
    /// in a terminal like own output, without one only on stderr so that a
    /// pipe receives just this shell's own output.
    fn show(
        &self,
        activity: Activity,
        running: bool,
        interrupted: bool,
        last_exit: i32,
    ) -> Option<i32> {
        match activity {
            Activity::Started { by, command } => eprintln!("[{by}] $ {command}"),
            Activity::Output { stream, bytes } => {
                if self.interactive && stream == Stream::Stdout {
                    write_out(std::io::stdout(), &bytes);
                } else {
                    write_out(std::io::stderr(), &bytes);
                }
            }
            Activity::Done { by, exit_code, .. } => eprintln!(
                "[{by}] exit {}",
                exit_code.map_or_else(|| "?".to_string(), |code| code.to_string())
            ),
            Activity::Notice(text) => eprintln!("[se] {text}"),
            Activity::Ended { exit_code, message } => {
                eprintln!("se: session ended: {message}");
                if !running {
                    return Some(if interrupted {
                        130
                    } else {
                        exit_code.unwrap_or(last_exit)
                    });
                }
            }
        }
        None
    }

    fn prompt(&self, continuation: bool) {
        if !self.interactive {
            return;
        }
        let text = if continuation {
            "> ".to_string()
        } else {
            let info = self.client.info();
            let sign = match self.dialect {
                Dialect::Posix => '$',
                Dialect::PowerShell => '>',
            };
            format!("{}:{}{sign} ", info.target, info.cwd.unwrap_or_default())
        };
        let mut stderr = std::io::stderr();
        let _ = stderr
            .write_all(text.as_bytes())
            .and_then(|()| stderr.flush());
    }

    /// The owner closes the session and waits for its end; an attached
    /// client detaches.
    fn finish(&self, code: i32) -> i32 {
        self.client.leave();
        if self.client.info().role == Role::Owner {
            self.client.wait_ended(Duration::from_secs(10));
        }
        code
    }
}

/// Joins continuation lines: a line ending in `\` (POSIX) or `` ` ``
/// (PowerShell) continues; the shell itself interprets the joined text.
/// `None` while the command is incomplete or empty.
fn complete(dialect: Dialect, partial: &mut Option<String>, line: String) -> Option<String> {
    let line = if let Some(stripped) = line.strip_suffix('\r') {
        stripped.to_string()
    } else {
        line
    };
    let continues = match dialect {
        Dialect::Posix => line.ends_with('\\'),
        Dialect::PowerShell => line.ends_with('`'),
    };
    let joined = match partial.take() {
        Some(mut previous) => {
            previous.push('\n');
            previous.push_str(&line);
            previous
        }
        None => line,
    };
    if continues {
        *partial = Some(joined);
        return None;
    }
    (!joined.trim().is_empty()).then_some(joined)
}

fn exit_code(outcome: &CommandOutcome) -> i32 {
    match outcome.status {
        OutcomeStatus::Completed => outcome.exit_code.unwrap_or(0),
        OutcomeStatus::Cancelled => 130,
        OutcomeStatus::Failed => outcome.exit_code.unwrap_or(125),
    }
}

/// Ctrl+C, an update stop request and standard input, as one event stream.
fn start_inputs(inputs: Sender<Input>) -> Result<(), String> {
    let interrupt = inputs.clone();
    ctrlc::set_handler(move || {
        let _ = interrupt.send(Input::Interrupt);
    })
    .map_err(|error| format!("Ctrl+C handler: {error}"))?;
    let update = inputs.clone();
    let baseline = crate::daemon::update_stop_marker();
    std::thread::Builder::new()
        .name("se-shell-update".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            let current = crate::daemon::update_stop_marker();
            if current.is_some() && current != baseline {
                let _ = update.send(Input::UpdateStop);
                return;
            }
        })
        .map_err(|error| format!("update watcher: {error}"))?;
    std::thread::Builder::new()
        .name("se-shell-stdin".into())
        .spawn(move || read_lines(inputs))
        .map_err(|error| format!("stdin reader: {error}"))?;
    Ok(())
}

fn read_lines(inputs: Sender<Input>) {
    let stdin = std::io::stdin();
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        match stdin.lock().read_until(b'\n', &mut bytes) {
            Ok(0) | Err(_) => {
                let _ = inputs.send(Input::Eof);
                return;
            }
            Ok(_) => {
                let text = String::from_utf8_lossy(&bytes);
                let line = text.strip_suffix('\n').unwrap_or(&text).to_string();
                if inputs.send(Input::Line(line)).is_err() {
                    return;
                }
            }
        }
    }
}

struct Printer {
    done: Sender<CommandOutcome>,
}

impl CommandSink for Printer {
    fn output(&self, stream: Stream, bytes: &[u8]) {
        match stream {
            Stream::Stdout => write_out(std::io::stdout(), bytes),
            Stream::Stderr => write_out(std::io::stderr(), bytes),
        }
    }

    fn finished(&self, outcome: CommandOutcome) {
        let _ = self.done.send(outcome);
    }
}

fn write_out(mut writer: impl Write, bytes: &[u8]) {
    let _ = writer.write_all(bytes).and_then(|()| writer.flush());
}

#[cfg(test)]
mod tests {
    use super::{complete, exit_code};
    use crate::cli::shell_session::{CommandOutcome, Dialect, OutcomeStatus};

    #[test]
    fn shell_session_task_cli_lines_continue_with_the_dialects_continuation_character() {
        let mut partial = None;
        assert_eq!(
            complete(Dialect::Posix, &mut partial, "echo a \\".into()),
            None
        );
        assert_eq!(
            complete(Dialect::Posix, &mut partial, "b\r".into()),
            Some("echo a \\\nb".into())
        );
        assert_eq!(complete(Dialect::Posix, &mut partial, "   ".into()), None);
        assert!(partial.is_none());
        assert_eq!(
            complete(Dialect::PowerShell, &mut partial, "dir C:\\".into()),
            Some("dir C:\\".into())
        );
        assert_eq!(
            complete(Dialect::PowerShell, &mut partial, "Get-Item `".into()),
            None
        );
        assert_eq!(
            complete(Dialect::PowerShell, &mut partial, "  x".into()),
            Some("Get-Item `\n  x".into())
        );
        let mut outcome = CommandOutcome {
            status: OutcomeStatus::Completed,
            exit_code: Some(4),
            cwd: None,
            session_ended: true,
            message: None,
        };
        assert_eq!(exit_code(&outcome), 4);
        outcome.status = OutcomeStatus::Cancelled;
        assert_eq!(exit_code(&outcome), 130);
        outcome.status = OutcomeStatus::Failed;
        outcome.exit_code = None;
        assert_eq!(exit_code(&outcome), 125);
    }
}
