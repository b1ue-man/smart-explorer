use super::config::SftpConfig;
use super::connection::{SftpConnection, SftpGeneration};
use super::io_adapters::{BlockingRead, BlockingWrite, SftpReader, SftpWriter};
use super::io_err;
use super::metadata::{basename, to_vfs};
use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use russh::client;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::OpenFlags;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;

const SSH_CHANNEL_OPEN_DEADLINE: Duration = Duration::from_secs(10);
const SSH_EXEC_REQUEST_DEADLINE: Duration = Duration::from_secs(10);
const SSH_EXEC_CAPTURE_DEADLINE: Duration = Duration::from_secs(30);
const SSH_EXEC_OUTPUT_LIMIT: usize = 64 * 1024;

#[derive(Clone)]
pub struct SftpBackend {
    rt: Arc<Runtime>,
    connection: Arc<SftpConnection>,
    root: String,
    /// Read by `url()` (UI display), consumed in the connect-UI step.
    #[allow(dead_code)]
    url: String,
}

impl SftpBackend {
    pub fn connect(cfg: SftpConfig) -> io::Result<SftpBackend> {
        let url = format!("sftp://{}@{}:{}{}", cfg.user, cfg.host, cfg.port, cfg.root);
        let root = cfg.root.clone();
        let connection = SftpConnection::connect(cfg)?;
        let rt = connection.runtime();
        Ok(SftpBackend {
            rt,
            connection,
            root,
            url,
        })
    }

    /// `sftp://user@host:port/root` for UI display (connect-UI step).
    #[allow(dead_code)]
    pub fn url(&self) -> String {
        self.url.clone()
    }

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

    fn open_session_channel(
        &self,
    ) -> io::Result<(Arc<SftpGeneration>, russh::Channel<client::Msg>)> {
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
                Ok(Ok(channel)) => return Ok((generation, channel)),
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
        unreachable!("bounded SSH channel-open attempts")
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

    fn safe_sftp<T>(
        &self,
        operation: impl Fn(&SftpGeneration) -> Result<T, SftpError>,
    ) -> io::Result<T> {
        self.safe_sftp_on(operation).map(|(_, value)| value)
    }

    fn safe_sftp_on<T>(
        &self,
        operation: impl Fn(&SftpGeneration) -> Result<T, SftpError>,
    ) -> io::Result<(Arc<SftpGeneration>, T)> {
        let mut generation = self.connection.current()?;
        for attempt in 0..2 {
            match operation(&generation) {
                Ok(value) => return Ok((generation, value)),
                Err(error) => {
                    let dead = self.connection.note_sftp_error(&generation, &error);
                    if attempt == 0 && dead {
                        generation = self.connection.current()?;
                        continue;
                    }
                    return Err(io_err(error));
                }
            }
        }
        unreachable!("bounded SFTP read attempts")
    }

    fn mutate_sftp<T>(
        &self,
        operation: impl FnOnce(&SftpGeneration) -> Result<T, SftpError>,
    ) -> io::Result<T> {
        let generation = self.connection.current()?;
        operation(&generation).map_err(|error| {
            self.connection.note_sftp_error(&generation, &error);
            io_err(error)
        })
    }
}

impl Backend for SftpBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }

    fn root_display(&self) -> String {
        self.root.clone()
    }

    fn state_identity(&self) -> String {
        self.url.clone()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let dir = self.safe_sftp(|generation| {
            self.rt
                .block_on(generation.sftp().read_dir(path.to_string()))
        })?;
        let mut out = Vec::new();
        for e in dir {
            let name = e.file_name();
            let meta = e.metadata();
            out.push(to_vfs(name, &meta));
        }
        Ok(out)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let meta = self.safe_sftp(|generation| {
            self.rt
                .block_on(generation.sftp().symlink_metadata(path.to_string()))
        })?;
        Ok(to_vfs(basename(path), &meta))
    }

    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        self.safe_sftp(|generation| {
            self.rt
                .block_on(generation.sftp().try_exists(path.to_string()))
        })
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let (generation, file) = self.safe_sftp_on(|generation| {
            self.rt.block_on(generation.sftp().open(path.to_string()))
        })?;
        Ok(Box::new(SftpReader {
            rt: self.rt.clone(),
            connection: self.connection.clone(),
            generation,
            path: path.to_string(),
            file,
            delivered: 0,
            retried: false,
        }))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let generation = self.connection.current()?;
        let file = self
            .rt
            .block_on(generation.sftp().create(path.to_string()))
            .map_err(|error| {
                self.connection.note_sftp_error(&generation, &error);
                io_err(error)
            })?;
        Ok(Box::new(SftpWriter {
            rt: self.rt.clone(),
            connection: self.connection.clone(),
            generation,
            file: Some(file),
        }))
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let generation = self.connection.current()?;
        let flags = OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE;
        let file = self
            .rt
            .block_on(generation.sftp().open_with_flags(path.to_string(), flags))
            .map_err(|error| {
                self.connection.note_sftp_error(&generation, &error);
                io_err(error)
            })?;
        Ok(Box::new(SftpWriter {
            rt: self.rt.clone(),
            connection: self.connection.clone(),
            generation,
            file: Some(file),
        }))
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.mutate_sftp(|generation| {
            self.rt
                .block_on(generation.sftp().rename(src.to_string(), dst.to_string()))
        })
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        // russh-sftp speaks SFTP v3. SSH_FXP_RENAME in that protocol must fail
        // when newpath already exists, so the request itself is the atomic
        // no-replace gate rather than a racy client-side existence probe.
        self.mutate_sftp(|generation| {
            self.rt
                .block_on(generation.sftp().rename(src.to_string(), dst.to_string()))
        })
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.mutate_sftp(|generation| {
            self.rt
                .block_on(generation.sftp().remove_file(path.to_string()))
        })
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.mutate_sftp(|generation| {
            self.rt
                .block_on(generation.sftp().remove_dir(path.to_string()))
        })
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let generation = self.connection.current()?;
        let absolute = path.starts_with('/');
        let parts: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let mut cur = String::new();
        for part in parts {
            if cur.is_empty() {
                if absolute {
                    cur.push('/');
                }
            } else {
                cur.push('/');
            }
            cur.push_str(&part);
            match self.rt.block_on(generation.sftp().create_dir(cur.clone())) {
                Ok(()) | Err(SftpError::Status(_)) => {}
                Err(error) => {
                    self.connection.note_sftp_error(&generation, &error);
                    return Err(io_err(error));
                }
            }
        }
        self.rt
            .block_on(generation.sftp().metadata(cur))
            .map(|_| ())
            .map_err(|error| {
                self.connection.note_sftp_error(&generation, &error);
                io_err(error)
            })
    }

    fn parallelism(&self) -> usize {
        // Conservative: one SFTP session, sequential remote walk. Safe default
        // until a real-server concurrency spike (plan §"open questions").
        1
    }

    fn staged_write_capabilities(&self, _root: &str) -> crate::vfs::StagedWriteCapabilities {
        crate::vfs::StagedWriteCapabilities {
            create: true,
            replace: false,
            namespace_replace: false,
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
