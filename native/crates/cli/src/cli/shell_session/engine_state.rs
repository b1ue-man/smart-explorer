//! The state machine of one session: the command queue, the command running
//! in the shell, its markers on both streams, and who receives what. It does
//! no I/O; `engine.rs` executes the returned deliveries in order.
use super::dialect;
use super::marker::{parse_stderr, parse_stdout, MarkerScanner, Piece};
use super::transports::TransportEnd;
use super::types::{
    Activity, CommandOutcome, CommandRequest, Delivery, Dialect, OutcomeStatus, OwnEvent,
    StdinMode, Stream,
};
use std::collections::VecDeque;

/// Commands waiting behind the running one. Each holds up to
/// `dialect::MAX_REQUEST_BYTES`; the bound keeps a flooding client from
/// growing the owner's memory.
const MAX_QUEUED: usize = 64;
/// Output that arrives while no command runs (background jobs) goes to the
/// next command; beyond this it is dropped and reported.
const MAX_HELD_BYTES: usize = 1024 * 1024;
/// Command text shown to the other participants.
const MAX_SHOWN_CHARS: usize = 4096;
/// Startup errors kept for the message of a failed start.
const MAX_STARTUP_ERRORS: usize = 4096;
/// Participant id of the session's own startup command.
const STARTUP: u64 = 0;

struct Pending {
    seq: u64,
    by: u64,
    by_label: String,
    tag: u64,
    request: CommandRequest,
}

struct Running {
    seq: u64,
    by: u64,
    by_label: String,
    tag: u64,
    interactive: bool,
    stdout_marker: Option<(Option<i32>, String)>,
    stderr_marker: bool,
}

impl Running {
    fn new(seq: u64, by: u64, by_label: String, tag: u64, interactive: bool) -> Self {
        Self {
            seq,
            by,
            by_label,
            tag,
            interactive,
            stdout_marker: None,
            stderr_marker: false,
        }
    }
}

pub(super) struct EngineState {
    dialect: Dialect,
    nonce: String,
    stdout: MarkerScanner,
    stderr: MarkerScanner,
    queue: VecDeque<Pending>,
    running: Option<Running>,
    next_seq: u64,
    ready: bool,
    closing: bool,
    cancelled_by: Option<String>,
    held: Vec<(Stream, Vec<u8>)>,
    held_bytes: usize,
    dropped_bytes: u64,
    startup_errors: Vec<u8>,
    startup_exit: Option<Option<i32>>,
    pub(super) cwd: Option<String>,
    /// Exit status and message once the shell is gone.
    pub(super) ended: Option<(Option<i32>, String)>,
}

impl EngineState {
    /// The state with its startup command (`startup`) running; the first
    /// delivery writes it to the shell.
    pub(super) fn new(
        dialect: Dialect,
        nonce: String,
        startup: &CommandRequest,
    ) -> Result<(Self, Vec<Delivery>), String> {
        let wrapper = dialect::wrap(dialect, &nonce, 0, startup)?;
        let state = Self {
            dialect,
            stdout: MarkerScanner::new(&nonce),
            stderr: MarkerScanner::new(&nonce),
            nonce,
            queue: VecDeque::new(),
            running: Some(Running::new(0, STARTUP, String::new(), 0, false)),
            next_seq: 1,
            ready: false,
            closing: false,
            cancelled_by: None,
            held: Vec::new(),
            held_bytes: 0,
            dropped_bytes: 0,
            startup_errors: Vec::new(),
            startup_exit: None,
            cwd: None,
            ended: None,
        };
        Ok((state, vec![Delivery::Write(wrapper.into_bytes())]))
    }

    pub(super) fn ready(&self) -> bool {
        self.ready
    }

    /// Why the session could not start: the startup command failed (for
    /// example a missing working directory) or the shell ended first.
    pub(super) fn startup_failure(&self) -> Option<String> {
        let errors = String::from_utf8_lossy(&self.startup_errors)
            .trim()
            .to_string();
        let detail = if errors.is_empty() {
            String::new()
        } else {
            format!(": {errors}")
        };
        if let Some(exit) = self.startup_exit {
            return (exit != Some(0)).then(|| {
                format!(
                    "the remote shell could not enter the requested working directory or environment (status {}){detail}",
                    exit.map_or_else(|| "unknown".to_string(), |code| code.to_string())
                )
            });
        }
        self.ended.as_ref().map(|(_, message)| {
            format!("the remote shell ended during its start ({message}){detail}")
        })
    }

    pub(super) fn submit(
        &mut self,
        by: u64,
        by_label: &str,
        tag: u64,
        mut request: CommandRequest,
    ) -> Result<Vec<Delivery>, String> {
        if let Some((_, message)) = &self.ended {
            return Err(format!("the session has ended: {message}"));
        }
        if self.closing {
            return Err("the session is closing".into());
        }
        if self.queue.len() >= MAX_QUEUED {
            return Err(format!(
                "{MAX_QUEUED} commands are already waiting in this session"
            ));
        }
        if self.dialect == Dialect::PowerShell && request.stdin == StdinMode::Interactive {
            // PowerShell reads its commands from standard input; typed input
            // cannot reach a running command there.
            request.stdin = StdinMode::Null;
        }
        dialect::check(self.dialect, &self.nonce, &request)?;
        let seq = self.next_seq;
        self.next_seq += 1;
        self.queue.push_back(Pending {
            seq,
            by,
            by_label: by_label.to_string(),
            tag,
            request,
        });
        Ok(self.dispatch())
    }

    /// Input for the requester's own interactive command.
    pub(super) fn input(&mut self, by: u64, bytes: &[u8]) -> Result<Vec<Delivery>, String> {
        match &self.running {
            Some(running) if running.by == by && running.interactive => {
                Ok(vec![Delivery::Write(bytes.to_vec())])
            }
            _ => Err("no interactive command of this client is running".into()),
        }
    }

    pub(super) fn output(&mut self, stream: Stream, bytes: &[u8]) -> Vec<Delivery> {
        let pieces = match stream {
            Stream::Stdout => self.stdout.feed(bytes),
            Stream::Stderr => self.stderr.feed(bytes),
        };
        let mut deliveries = Vec::new();
        for piece in pieces {
            match piece {
                Piece::Data(data) => deliveries.extend(self.data(stream, data)),
                Piece::Marker(body) => deliveries.extend(self.marker(stream, &body)),
            }
        }
        deliveries
    }

    /// Records who cancelled; the transport cancel follows.
    pub(super) fn cancelled(&mut self, by: &str) {
        if self.cancelled_by.is_none() {
            self.cancelled_by = Some(by.to_string());
        }
    }

    /// `exit` for an idle session; `None` while a command runs (the caller
    /// cancels instead) or once the session ended.
    pub(super) fn close_input(&mut self) -> Option<Vec<u8>> {
        if self.ended.is_some() || self.running.is_some() {
            return None;
        }
        self.closing = true;
        Some(dialect::EXIT.to_vec())
    }

    /// A participant left: its waiting commands are dropped; a running one
    /// finishes without a receiver.
    pub(super) fn forget(&mut self, by: u64) {
        self.queue.retain(|pending| pending.by != by);
    }

    pub(super) fn ended(&mut self, end: TransportEnd) -> Vec<Delivery> {
        let mut deliveries = Vec::new();
        let tail = self.stdout.flush();
        if !tail.is_empty() {
            deliveries.extend(self.data(Stream::Stdout, tail));
        }
        let tail = self.stderr.flush();
        if !tail.is_empty() {
            deliveries.extend(self.data(Stream::Stderr, tail));
        }
        let cancelled = end.cancelled || self.cancelled_by.is_some();
        let message = end_message(self.cancelled_by.as_deref(), &end);
        self.ended = Some((end.exit_code, message.clone()));
        if let Some(running) = self.running.take() {
            if running.by != STARTUP {
                let status = if cancelled {
                    OutcomeStatus::Cancelled
                } else if end.exit_code.is_some() && end.error.is_none() {
                    OutcomeStatus::Completed
                } else {
                    OutcomeStatus::Failed
                };
                deliveries.push(done(
                    running.by,
                    running.tag,
                    outcome(
                        status,
                        end.exit_code,
                        self.cwd.clone(),
                        true,
                        Some(message.clone()),
                    ),
                ));
            }
        }
        for pending in self.queue.drain(..) {
            let reason = format!("the command did not run: {message}");
            let failed = outcome(
                OutcomeStatus::Failed,
                None,
                self.cwd.clone(),
                true,
                Some(reason),
            );
            deliveries.push(done(pending.by, pending.tag, failed));
        }
        deliveries.push(Delivery::Activity {
            except: None,
            activity: Activity::Ended {
                exit_code: end.exit_code,
                message,
            },
        });
        deliveries
    }

    fn dispatch(&mut self) -> Vec<Delivery> {
        let mut deliveries = Vec::new();
        while self.ready && self.running.is_none() && self.ended.is_none() {
            let Some(next) = self.queue.pop_front() else {
                break;
            };
            let wrapper = match dialect::wrap(self.dialect, &self.nonce, next.seq, &next.request) {
                Ok(wrapper) => wrapper,
                Err(message) => {
                    let failed = outcome(
                        OutcomeStatus::Failed,
                        None,
                        self.cwd.clone(),
                        false,
                        Some(message),
                    );
                    deliveries.push(done(next.by, next.tag, failed));
                    continue;
                }
            };
            deliveries.push(Delivery::Write(wrapper.into_bytes()));
            deliveries.push(Delivery::Activity {
                except: Some(next.by),
                activity: Activity::Started {
                    by: next.by_label.clone(),
                    command: shown(&next.request.command),
                },
            });
            let interactive = next.request.stdin == StdinMode::Interactive;
            self.running = Some(Running::new(
                next.seq,
                next.by,
                next.by_label,
                next.tag,
                interactive,
            ));
            if self.dropped_bytes > 0 {
                deliveries.push(Delivery::Activity {
                    except: None,
                    activity: Activity::Notice(format!(
                        "{} bytes of output between commands were dropped",
                        self.dropped_bytes
                    )),
                });
                self.dropped_bytes = 0;
            }
            // Output that arrived between commands belongs to this one.
            self.held_bytes = 0;
            for (stream, bytes) in std::mem::take(&mut self.held) {
                deliveries.extend(self.route(stream, bytes));
            }
        }
        deliveries
    }

    fn data(&mut self, stream: Stream, mut data: Vec<u8>) -> Vec<Delivery> {
        match &self.running {
            Some(running) if running.by == STARTUP => {
                if stream == Stream::Stderr {
                    let room = MAX_STARTUP_ERRORS.saturating_sub(self.startup_errors.len());
                    data.truncate(room);
                    self.startup_errors.extend_from_slice(&data);
                }
                Vec::new()
            }
            Some(_) => self.route(stream, data),
            None => {
                let room = MAX_HELD_BYTES.saturating_sub(self.held_bytes);
                if data.len() > room {
                    self.dropped_bytes += (data.len() - room) as u64;
                    data.truncate(room);
                }
                if !data.is_empty() {
                    self.held_bytes += data.len();
                    self.held.push((stream, data));
                }
                Vec::new()
            }
        }
    }

    fn route(&self, stream: Stream, bytes: Vec<u8>) -> Vec<Delivery> {
        let Some(running) = &self.running else {
            return Vec::new();
        };
        vec![
            Delivery::Activity {
                except: Some(running.by),
                activity: Activity::Output {
                    stream,
                    bytes: bytes.clone(),
                },
            },
            Delivery::Own {
                to: running.by,
                event: OwnEvent::Output {
                    tag: running.tag,
                    stream,
                    bytes,
                },
            },
        ]
    }

    fn marker(&mut self, stream: Stream, body: &str) -> Vec<Delivery> {
        let Some(running) = self.running.as_mut() else {
            return Vec::new();
        };
        match stream {
            Stream::Stdout => match parse_stdout(body) {
                Some((seq, exit, cwd)) if seq == running.seq => {
                    running.stdout_marker = Some((exit, cwd));
                }
                _ => return Vec::new(),
            },
            Stream::Stderr => match parse_stderr(body) {
                Some(seq) if seq == running.seq => running.stderr_marker = true,
                _ => return Vec::new(),
            },
        }
        if !running.stderr_marker || running.stdout_marker.is_none() {
            return Vec::new();
        }
        let Some(finished) = self.running.take() else {
            return Vec::new();
        };
        let (exit_code, cwd) = finished.stdout_marker.unwrap_or((None, String::new()));
        self.cwd = Some(cwd.clone());
        let mut deliveries = Vec::new();
        if finished.by == STARTUP {
            self.ready = true;
            self.startup_exit = Some(exit_code);
        } else {
            deliveries.push(Delivery::Activity {
                except: Some(finished.by),
                activity: Activity::Done {
                    by: finished.by_label.clone(),
                    exit_code,
                    cwd: Some(cwd.clone()),
                },
            });
            let completed = outcome(OutcomeStatus::Completed, exit_code, Some(cwd), false, None);
            deliveries.push(done(finished.by, finished.tag, completed));
        }
        deliveries.extend(self.dispatch());
        deliveries
    }
}

fn end_message(cancelled_by: Option<&str>, end: &TransportEnd) -> String {
    if let Some(by) = cancelled_by {
        format!("the session was cancelled by {by}")
    } else if end.cancelled {
        "the session was cancelled".into()
    } else if let Some(error) = &end.error {
        error.clone()
    } else if let Some(code) = end.exit_code {
        format!("the shell exited with status {code}")
    } else {
        "the shell ended".into()
    }
}

fn done(to: u64, tag: u64, outcome: CommandOutcome) -> Delivery {
    Delivery::Own {
        to,
        event: OwnEvent::Done { tag, outcome },
    }
}

fn outcome(
    status: OutcomeStatus,
    exit_code: Option<i32>,
    cwd: Option<String>,
    session_ended: bool,
    message: Option<String>,
) -> CommandOutcome {
    CommandOutcome {
        status,
        exit_code,
        cwd,
        session_ended,
        message,
    }
}

/// The command as others see it: bounded, without control characters that
/// could steer their terminal.
fn shown(command: &str) -> String {
    let mut text: String = command
        .chars()
        .take(MAX_SHOWN_CHARS)
        .map(|ch| {
            if ch.is_control() && ch != '\n' && ch != '\t' {
                '\u{fffd}'
            } else {
                ch
            }
        })
        .collect();
    if command.chars().count() > MAX_SHOWN_CHARS {
        text.push('…');
    }
    text
}

#[cfg(test)]
#[path = "engine_state_tests.rs"]
mod tests;
