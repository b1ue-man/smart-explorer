//! User commands on the authenticated SSH session of a saved SFTP remote
//! (`se exec @label`, the MCP `exec` tool). Each command gets its own exec
//! channel; stdout and stderr stay separate, stdin is streamed, and the exit
//! status or signal is reported. Cancellation sends the RFC 4254 `signal`
//! request TERM, then KILL after a grace period, then closes the channel.
//! OpenSSH 7.9+ delivers these signals with `killpg` to the command's process
//! group (`session.c::session_signal_req`); servers without signal support
//! only see the channel close. See docs/AI_AGENT_PLUGIN.md.
//!
//! `run_channel` drives any session channel whose program request is
//! already sent; `terminal.rs` uses it for interactive channels with a PTY.
use super::backend::SftpBackend;
use russh::{client, ChannelMsg, Sig};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Grace period after TERM and again after KILL before the channel is closed
/// regardless; the same default as `docker stop`.
const CANCEL_GRACE: Duration = Duration::from_secs(10);
/// Output events waiting for a slow reader. A full queue stops reading the
/// channel, so SSH flow control throttles the remote process instead of
/// buffering its output in memory.
const EVENT_QUEUE: usize = 64;

#[derive(Debug)]
pub enum SshCommandEvent {
    /// The server accepted the exec request; the command runs.
    Started,
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// Always the last event.
    Finished(SshCommandEnd),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshCommandEnd {
    pub exit_status: Option<u32>,
    /// Signal name reported by the server (`TERM`, `KILL`, …).
    pub signal: Option<String>,
    /// A cancellation was requested before the command ended.
    pub cancelled: bool,
    /// Rejected exec request, transport loss or an unanswered cancellation.
    pub error: Option<String>,
}

/// A running remote command. Dropping it keeps the command running only as
/// long as the event queue drains; use [`SshCommandControl::cancel`] to stop it.
pub struct SshCommand {
    events: mpsc::Receiver<SshCommandEvent>,
    control: SshCommandControl,
    /// Keeps the SSH session of the command alive.
    _backend: Arc<SftpBackend>,
}

#[derive(Clone)]
pub struct SshCommandControl {
    tx: mpsc::UnboundedSender<Control>,
}

enum Control {
    Stdin(Vec<u8>),
    Eof,
    Cancel,
    Resize { cols: u32, rows: u32 },
}

impl SshCommandControl {
    pub fn stdin(&self, bytes: &[u8]) -> io::Result<()> {
        self.send(Control::Stdin(bytes.to_vec()))
    }

    pub fn eof(&self) -> io::Result<()> {
        self.send(Control::Eof)
    }

    /// Idempotent; a command that already ended ignores it.
    pub fn cancel(&self) {
        let _ = self.tx.send(Control::Cancel);
    }

    /// Sends an SSH window-change; only meaningful on a terminal channel.
    pub fn resize(&self, cols: u32, rows: u32) -> io::Result<()> {
        self.send(Control::Resize { cols, rows })
    }

    fn send(&self, control: Control) -> io::Result<()> {
        self.tx
            .send(control)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "remote command has ended"))
    }
}

impl SshCommand {
    pub fn control(&self) -> SshCommandControl {
        self.control.clone()
    }

    /// Blocks until the next event; `None` after `Finished` was delivered.
    /// Must not be called from inside a tokio runtime.
    pub fn next_event(&mut self) -> Option<SshCommandEvent> {
        self.events.blocking_recv()
    }
}

impl SftpBackend {
    /// Starts `command` through the remote user's login shell, exactly as
    /// `ssh host 'command'` would.
    pub fn start_command(self: Arc<Self>, command: &str) -> io::Result<SshCommand> {
        let (generation, channel) = self.open_session_channel()?;
        self.request_exec(&generation, &channel, true, command)?;
        Ok(run_channel(self, channel))
    }
}

/// Starts the pump for a session channel whose program request (`exec` or
/// `shell`, with reply requested) was just sent, and returns the handle.
pub(super) fn run_channel(
    backend: Arc<SftpBackend>,
    channel: russh::Channel<client::Msg>,
) -> SshCommand {
    let (event_tx, events) = mpsc::channel(EVENT_QUEUE);
    let (control_tx, control_rx) = mpsc::unbounded_channel();
    backend.rt.spawn(pump(channel, event_tx, control_rx));
    SshCommand {
        events,
        control: SshCommandControl { tx: control_tx },
        _backend: backend,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CancelStage {
    Running,
    Terminated,
    Killed,
}

async fn pump(
    mut channel: russh::Channel<client::Msg>,
    events: mpsc::Sender<SshCommandEvent>,
    mut control: mpsc::UnboundedReceiver<Control>,
) {
    let mut end = SshCommandEnd::default();
    let mut started = false;
    let mut control_open = true;
    let mut stage = CancelStage::Running;
    let mut deadline = tokio::time::Instant::now();
    loop {
        tokio::select! {
            message = channel.wait() => match message {
                Some(ChannelMsg::Success) if !started => {
                    started = true;
                    let _ = events.send(SshCommandEvent::Started).await;
                }
                Some(ChannelMsg::Failure) if !started => {
                    end.error = Some(
                        "the SSH server refused to run commands for this login (exec request rejected)"
                            .to_string(),
                    );
                    break;
                }
                Some(ChannelMsg::Data { data }) => {
                    let _ = events.send(SshCommandEvent::Stdout(data.to_vec())).await;
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    let _ = events.send(SshCommandEvent::Stderr(data.to_vec())).await;
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => end.exit_status = Some(exit_status),
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                    end.signal = Some(signal_name_text(&signal_name));
                }
                Some(ChannelMsg::Close) | None => break,
                _ => {}
            },
            request = control.recv(), if control_open => match request {
                Some(Control::Stdin(bytes)) => {
                    let _ = channel.data(&bytes[..]).await;
                }
                Some(Control::Eof) => {
                    let _ = channel.eof().await;
                }
                Some(Control::Cancel) if stage == CancelStage::Running => {
                    end.cancelled = true;
                    stage = CancelStage::Terminated;
                    deadline = tokio::time::Instant::now() + CANCEL_GRACE;
                    let _ = channel.signal(Sig::TERM).await;
                }
                Some(Control::Cancel) => {}
                Some(Control::Resize { cols, rows }) => {
                    // RFC 4254 6.7: window-change has no reply.
                    let _ = channel.window_change(cols, rows, 0, 0).await;
                }
                None => control_open = false,
            },
            _ = tokio::time::sleep_until(deadline), if stage != CancelStage::Running => {
                if stage == CancelStage::Terminated {
                    stage = CancelStage::Killed;
                    deadline = tokio::time::Instant::now() + CANCEL_GRACE;
                    let _ = channel.signal(Sig::KILL).await;
                } else {
                    end.error = Some(
                        "the remote command did not end after TERM and KILL; the SSH channel was closed and the server may keep it running"
                            .to_string(),
                    );
                    break;
                }
            }
        }
    }
    if end.exit_status.is_none() && end.signal.is_none() && end.error.is_none() && !end.cancelled {
        end.error = Some("the SSH channel closed without an exit status".to_string());
    }
    let _ = channel.close().await;
    let _ = events.send(SshCommandEvent::Finished(end)).await;
}

fn signal_name_text(signal: &Sig) -> String {
    match signal {
        Sig::Custom(name) => name.clone(),
        other => format!("{other:?}"),
    }
}

/// POSIX (Linux) signal number of an SSH signal name, for `128 + n` exit
/// codes; `None` for names without a portable number.
pub fn signal_number(name: &str) -> Option<i32> {
    Some(match name.trim_end_matches("@openssh.com") {
        "HUP" => 1,
        "INT" => 2,
        "QUIT" => 3,
        "ILL" => 4,
        "ABRT" => 6,
        "FPE" => 8,
        "KILL" => 9,
        "USR1" => 10,
        "SEGV" => 11,
        "USR2" => 12,
        "PIPE" => 13,
        "ALRM" => 14,
        "TERM" => 15,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{signal_name_text, signal_number};
    use russh::Sig;

    #[test]
    fn mcp_task_ssh_signal_names_map_to_posix_numbers() {
        assert_eq!(signal_name_text(&Sig::TERM), "TERM");
        assert_eq!(
            signal_name_text(&Sig::Custom("INFO@openssh.com".into())),
            "INFO@openssh.com"
        );
        assert_eq!(signal_number("TERM"), Some(15));
        assert_eq!(signal_number("KILL"), Some(9));
        assert_eq!(signal_number("INFO@openssh.com"), None);
    }
}
