//! Remote commands and extra session channels on the backend's SSH
//! connection: one-shot commands for the remote-agent deploy, the agent's
//! framed stdio streams, and the channel opening shared with the SFTP
//! channel pool and `posix-rename`.
use super::backend::SftpBackend;
use super::connection::SftpGeneration;
use super::io_adapters::{BlockingRead, BlockingWrite};
use super::io_err;
use russh::client;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::Duration;

const SSH_CHANNEL_OPEN_DEADLINE: Duration = Duration::from_secs(10);
const SSH_EXEC_REQUEST_DEADLINE: Duration = Duration::from_secs(10);
const SSH_EXEC_CAPTURE_DEADLINE: Duration = Duration::from_secs(30);
const SSH_EXEC_OUTPUT_LIMIT: usize = 64 * 1024;

/// Result of asking the server for one more session channel.
pub(super) enum ChannelOpen {
    Opened(Arc<SftpGeneration>, russh::Channel<client::Msg>),
    /// The server refused the channel (`SSH_MSG_CHANNEL_OPEN_FAILURE`); OpenSSH
    /// answers this way once `MaxSessions` channels are open.
    Refused(russh::ChannelOpenFailure),
}

impl SftpBackend {
    /// Run a one-shot remote command and capture its stdout — used by the SSH
    /// remote-agent deploy (`uname -sm`, `$HOME`, the agent `--version` probe,
    /// `mv`/`chmod`, `sha256sum`, cleanup). Opens a fresh exec channel on the
    /// already-authenticated session. See `docs/SSH_AGENT_PLAN.md`.
    pub fn exec_capture(&self, cmd: &str) -> io::Result<String> {
        let (generation, mut ch) = self.open_session_channel()?;
        self.request_exec(&generation, &ch, true, cmd)?;
        let capture = self.rt.block_on(async {
            tokio::time::timeout(SSH_EXEC_CAPTURE_DEADLINE, capture_exec(&mut ch)).await
        });
        let capture = match capture {
            Ok(result) => result?,
            Err(_) => {
                let error = deadline_error("SSH remote command completion");
                self.connection.note_io_error(&generation, &error);
                return Err(error);
            }
        };
        if generation.session().is_closed() {
            self.connection.mark_stale(&generation);
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "SSH transport closed while waiting for command output",
            ));
        }
        capture.finish()
    }

    /// Exec `cmd` and return blocking read/write halves over its stdio, for the
    /// agent's framed request/response protocol (the agent runs `--serve`).
    pub fn open_exec_streams(
        &self,
        cmd: &str,
    ) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        let (generation, ch) = self.open_session_channel()?;
        self.request_exec(&generation, &ch, false, cmd)?;
        let stream = ch.into_stream();
        let (rd, wr) = tokio::io::split(stream);
        let r: Box<dyn Read + Send> = Box::new(BlockingRead {
            rt: self.rt.clone(),
            inner: Some(rd),
        });
        let w: Box<dyn Write + Send> = Box::new(BlockingWrite {
            rt: self.rt.clone(),
            inner: Some(wr),
        });
        Ok((r, w))
    }

    pub(super) fn open_session_channel(
        &self,
    ) -> io::Result<(Arc<SftpGeneration>, russh::Channel<client::Msg>)> {
        match self.try_open_session_channel()? {
            ChannelOpen::Opened(generation, channel) => Ok((generation, channel)),
            ChannelOpen::Refused(reason) => Err(io_err(russh::Error::ChannelOpenFailure(reason))),
        }
    }

    /// Opens one more session channel; a proven dead transport is replaced
    /// and the open tried once more. A refusal is reported, not retried.
    pub(super) fn try_open_session_channel(&self) -> io::Result<ChannelOpen> {
        let mut generation = self.connection.current()?;
        for attempt in 0..2 {
            let opened = self.rt.block_on(async {
                tokio::time::timeout(
                    SSH_CHANNEL_OPEN_DEADLINE,
                    generation.session().channel_open_session(),
                )
                .await
            });
            match opened {
                Ok(Ok(channel)) => return Ok(ChannelOpen::Opened(generation, channel)),
                Ok(Err(russh::Error::ChannelOpenFailure(reason))) => {
                    return Ok(ChannelOpen::Refused(reason))
                }
                Ok(Err(error)) => {
                    let dead = self.connection.note_ssh_error(&generation, &error);
                    if attempt == 0 && dead {
                        generation = self.connection.current()?;
                        continue;
                    }
                    return Err(io_err(error));
                }
                Err(_) => {
                    let error = deadline_error("SSH session channel open");
                    self.connection.note_io_error(&generation, &error);
                    return Err(error);
                }
            }
        }
        Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "SSH-Kanal ließ sich auch nach dem Neuverbinden nicht öffnen",
        ))
    }

    fn request_exec(
        &self,
        generation: &Arc<SftpGeneration>,
        channel: &russh::Channel<client::Msg>,
        want_reply: bool,
        cmd: &str,
    ) -> io::Result<()> {
        let requested = self.rt.block_on(async {
            tokio::time::timeout(
                SSH_EXEC_REQUEST_DEADLINE,
                channel.exec(want_reply, cmd.as_bytes().to_vec()),
            )
            .await
        });
        match requested {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.connection.note_ssh_error(generation, &error);
                Err(io_err(error))
            }
            Err(_) => {
                let error = deadline_error("SSH exec request");
                self.connection.note_io_error(generation, &error);
                Err(error)
            }
        }
    }
}

#[derive(Default)]
pub(super) struct CapturedExec {
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) stdout_truncated: bool,
    pub(super) stderr_truncated: bool,
    pub(super) exit_status: Option<u32>,
    pub(super) exit_signal: Option<String>,
}

impl CapturedExec {
    pub(super) fn finish(self) -> io::Result<String> {
        if let Some(signal) = self.exit_signal.as_deref() {
            return Err(io::Error::other(format!(
                "SSH remote command terminated by signal {signal}{}",
                self.stderr_context()
            )));
        }
        let status = self.exit_status.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "SSH remote command closed without an exit status{}",
                    self.stderr_context()
                ),
            )
        })?;
        if status != 0 {
            return Err(io::Error::other(format!(
                "SSH remote command exited with status {status}{}",
                self.stderr_context()
            )));
        }
        if self.stdout_truncated {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SSH remote command stdout exceeded its 64 KiB limit",
            ));
        }
        Ok(String::from_utf8_lossy(&self.stdout).trim().to_string())
    }

    fn stderr_context(&self) -> String {
        let stderr = String::from_utf8_lossy(&self.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            String::new()
        } else if self.stderr_truncated {
            format!(": {stderr} [truncated at 64 KiB]")
        } else {
            format!(": {stderr}")
        }
    }
}

async fn capture_exec(channel: &mut russh::Channel<client::Msg>) -> io::Result<CapturedExec> {
    let mut capture = CapturedExec::default();
    loop {
        match channel.wait().await {
            Some(russh::ChannelMsg::Data { data }) => {
                capture.stdout_truncated |=
                    append_bounded(&mut capture.stdout, &data, SSH_EXEC_OUTPUT_LIMIT);
            }
            Some(russh::ChannelMsg::ExtendedData { data, .. }) => {
                capture.stderr_truncated |=
                    append_bounded(&mut capture.stderr, &data, SSH_EXEC_OUTPUT_LIMIT);
            }
            Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                capture.exit_status = Some(exit_status);
            }
            Some(russh::ChannelMsg::ExitSignal {
                signal_name,
                error_message,
                ..
            }) => {
                let error_message: String = error_message.chars().take(1024).collect();
                capture.exit_signal = Some(if error_message.trim().is_empty() {
                    format!("{signal_name:?}")
                } else {
                    format!("{signal_name:?} ({error_message})")
                });
            }
            Some(russh::ChannelMsg::Failure) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "SSH server rejected the remote exec request",
                ));
            }
            Some(russh::ChannelMsg::Close) | None => return Ok(capture),
            _ => {}
        }
    }
}

pub(super) fn append_bounded(target: &mut Vec<u8>, source: &[u8], limit: usize) -> bool {
    let remaining = limit.saturating_sub(target.len());
    target.extend_from_slice(&source[..source.len().min(remaining)]);
    source.len() > remaining
}

fn deadline_error(stage: &str) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, format!("{stage} timed out"))
}
