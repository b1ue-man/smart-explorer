use super::config::SftpConfig;
use super::connection::{SftpConnection, SftpGeneration};
use super::io_adapters::{BlockingRead, BlockingWrite, SftpReader, SftpWriter};
use super::io_err;
use super::metadata::{basename, to_vfs};
use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use russh::client;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{OpenFlags, Packet, StatusCode};
use std::io::{self, Read, Write};
use std::sync::Arc;
use tokio::runtime::Runtime;

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
        self.rt.block_on(async {
            if let Err(error) = ch.exec(true, cmd).await {
                self.connection.note_ssh_error(&generation, &error);
                return Err(io_err(error));
            }
            let mut out = Vec::new();
            loop {
                match ch.wait().await {
                    Some(russh::ChannelMsg::Data { data }) => out.extend_from_slice(&data),
                    Some(russh::ChannelMsg::Close) | None => break,
                    _ => {} // ExtendedData (stderr), Eof, ExitStatus, … → ignore
                }
            }
            if generation.session().is_closed() {
                self.connection.mark_stale(&generation);
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "SSH transport closed while waiting for command output",
                ));
            }
            Ok::<_, io::Error>(String::from_utf8_lossy(&out).trim().to_string())
        })
    }

    /// Exec `cmd` and return blocking read/write halves over its stdio, for the
    /// agent's framed request/response protocol (the agent runs `--serve`).
    pub fn open_exec_streams(
        &self,
        cmd: &str,
    ) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        let (generation, ch) = self.open_session_channel()?;
        let stream = self.rt.block_on(async {
            if let Err(error) = ch.exec(false, cmd).await {
                self.connection.note_ssh_error(&generation, &error);
                return Err(io_err(error));
            }
            Ok::<_, io::Error>(ch.into_stream())
        })?;
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
            match self
                .rt
                .block_on(generation.session().channel_open_session())
            {
                Ok(channel) => return Ok((generation, channel)),
                Err(error) => {
                    let dead = self.connection.note_ssh_error(&generation, &error);
                    if attempt == 0 && dead {
                        generation = self.connection.current()?;
                        continue;
                    }
                    return Err(io_err(error));
                }
            }
        }
        unreachable!("bounded SSH channel-open attempts")
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

    fn posix_rename(&self, source: &str, destination: &str) -> io::Result<()> {
        const POSIX_RENAME: &str = "posix-rename@openssh.com";

        let generation = self.connection.current()?;
        let session = generation.posix_rename().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "SFTP server did not advertise posix-rename@openssh.com version 1",
            )
        })?;
        let data = encode_path_pair(source, destination)?;
        self.rt
            .block_on(session.extended(POSIX_RENAME, data))
            .and_then(|packet| match packet {
                Packet::Status(status) if status.status_code == StatusCode::Ok => Ok(()),
                Packet::Status(status) => Err(SftpError::Status(status)),
                _ => Err(SftpError::UnexpectedPacket),
            })
            .map_err(|error| {
                // A missing mutation reply is ambiguous and must never be
                // replayed on a replacement connection.
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
        if self
            .connection
            .current()
            .is_ok_and(|generation| generation.posix_rename().is_some())
        {
            self.posix_rename(src, dst)
        } else {
            self.mutate_sftp(|generation| {
                self.rt
                    .block_on(generation.sftp().rename(src.to_string(), dst.to_string()))
            })
        }
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

    fn rename_overwrites(&self) -> bool {
        self.connection
            .current()
            .is_ok_and(|generation| generation.posix_rename().is_some())
    }

    fn staged_write_capabilities(&self, _root: &str) -> crate::vfs::StagedWriteCapabilities {
        let replace = self.rename_overwrites();
        crate::vfs::StagedWriteCapabilities {
            create: true,
            replace,
            namespace_replace: replace,
        }
    }
}

fn encode_path_pair(source: &str, destination: &str) -> io::Result<Vec<u8>> {
    let mut encoded = Vec::with_capacity(source.len() + destination.len() + 8);
    for path in [source, destination] {
        let length = u32::try_from(path.len()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "SFTP rename path is too long")
        })?;
        encoded.extend_from_slice(&length.to_be_bytes());
        encoded.extend_from_slice(path.as_bytes());
    }
    Ok(encoded)
}
