use super::config::SftpConfig;
use super::io_adapters::{BlockingRead, BlockingWrite, SftpReader, SftpWriter};
use super::io_err;
use super::metadata::{basename, to_vfs};
use super::session::{connect_async, Client};
use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use russh::client;
use russh_sftp::client::SftpSession;
use std::io::{self, Read, Write};
use std::sync::Arc;
use tokio::runtime::Runtime;

pub struct SftpBackend {
    rt: Arc<Runtime>,
    // Kept alive so the encrypted connection (and its background task) survive.
    _session: client::Handle<Client>,
    sftp: Arc<SftpSession>,
    root: String,
    /// Read by `url()` (UI display), consumed in the connect-UI step.
    #[allow(dead_code)]
    url: String,
}

impl SftpBackend {
    pub fn connect(cfg: SftpConfig) -> io::Result<SftpBackend> {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(io_err)?,
        );
        let url = format!("sftp://{}@{}:{}{}", cfg.user, cfg.host, cfg.port, cfg.root);
        let root = cfg.root.clone();
        let (session, sftp) = rt.block_on(connect_async(cfg))?;
        Ok(SftpBackend {
            rt,
            _session: session,
            sftp: Arc::new(sftp),
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
        self.rt.block_on(async {
            let mut ch = self._session.channel_open_session().await.map_err(io_err)?;
            ch.exec(true, cmd).await.map_err(io_err)?;
            let mut out = Vec::new();
            loop {
                match ch.wait().await {
                    Some(russh::ChannelMsg::Data { data }) => out.extend_from_slice(&data),
                    Some(russh::ChannelMsg::Close) | None => break,
                    _ => {} // ExtendedData (stderr), Eof, ExitStatus, … → ignore
                }
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
        let stream = self.rt.block_on(async {
            let ch = self._session.channel_open_session().await.map_err(io_err)?;
            ch.exec(false, cmd).await.map_err(io_err)?;
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
}

impl Backend for SftpBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }

    fn root_display(&self) -> String {
        self.root.clone()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let p = path.to_string();
        let dir = rt
            .block_on(async move { sftp.read_dir(p).await })
            .map_err(io_err)?;
        let mut out = Vec::new();
        for e in dir {
            let name = e.file_name();
            let meta = e.metadata();
            out.push(to_vfs(name, &meta));
        }
        Ok(out)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let p = path.to_string();
        let meta = rt
            .block_on(async move { sftp.symlink_metadata(p).await })
            .map_err(io_err)?;
        Ok(to_vfs(basename(path), &meta))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let p = path.to_string();
        let file = rt
            .block_on(async move { sftp.open(p).await })
            .map_err(io_err)?;
        Ok(Box::new(SftpReader {
            rt: self.rt.clone(),
            file,
        }))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let p = path.to_string();
        let file = rt
            .block_on(async move { sftp.create(p).await })
            .map_err(io_err)?;
        Ok(Box::new(SftpWriter {
            rt: self.rt.clone(),
            file: Some(file),
        }))
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let (s, d) = (src.to_string(), dst.to_string());
        rt.block_on(async move { sftp.rename(s, d).await })
            .map_err(io_err)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let p = path.to_string();
        rt.block_on(async move { sftp.remove_file(p).await })
            .map_err(io_err)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let p = path.to_string();
        rt.block_on(async move { sftp.remove_dir(p).await })
            .map_err(io_err)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let absolute = path.starts_with('/');
        let parts: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        rt.block_on(async move {
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
                // ignore "already exists"; final existence is verified below.
                let _ = sftp.create_dir(cur.clone()).await;
            }
            sftp.metadata(cur).await.map(|_| ()).map_err(io_err)
        })
    }

    fn parallelism(&self) -> usize {
        // Conservative: one SFTP session, sequential remote walk. Safe default
        // until a real-server concurrency spike (plan §"open questions").
        1
    }
}
