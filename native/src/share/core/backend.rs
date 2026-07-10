use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};

use super::core::eio;
use super::framing::{recv_ctrl, TAG_DATA};
use super::identity::ShareIdentity;
use super::io_deadline;
use super::session::relation_kind_id;
use super::types::{ExecRequest, ExecResult, PeerEndpoint, ShareEvent, ShareStatus};
use super::wire::{Ctrl, FsMeta, FsRequest, FsResponse};

pub(super) use super::framing::{recv_resp, recv_tagged, reply, send_ctrl};
pub(crate) use super::node::ShareIrohNode;

pub struct PeerBackend {
    pub(super) endpoint: PeerEndpoint,
    pub(super) identity: ShareIdentity,
    pub(super) node: Arc<ShareIrohNode>,
}

impl PeerBackend {
    pub(crate) fn new(
        endpoint: PeerEndpoint,
        identity: ShareIdentity,
        node: Arc<ShareIrohNode>,
    ) -> Self {
        Self {
            endpoint,
            identity,
            node,
        }
    }

    pub(crate) fn probe_root(&self) -> io::Result<Vec<VfsMeta>> {
        self.list_dir("/")
    }

    pub(crate) fn transport_status(&self) -> ShareStatus {
        self.node
            .session_transport(&self.endpoint)
            .map(|transport| match transport {
                "relay" => ShareStatus::ConnectedRelay,
                "direct" => ShareStatus::ConnectedDirect,
                _ => ShareStatus::Connected,
            })
            .unwrap_or(ShareStatus::Connected)
    }

    fn request(&self, req: FsRequest) -> io::Result<FsResponse> {
        let operation = fs_request_label(&req);
        let started = Instant::now();
        let (mut send, mut recv) = self.node.open_stream(&self.endpoint, &self.identity)?;
        let result = self
            .node
            .block_on(io_deadline::run("peer filesystem request", async {
                send_ctrl(&mut send, &Ctrl::Fs { req }).await?;
                recv_resp(&mut recv).await
            }));
        if result.is_err() {
            io_deadline::abort(&mut send, &mut recv);
        }
        let response = result?;
        let _ = self.node.ev.send(ShareEvent::Status(format!(
            "Share-Op {operation}: {} ms, {}",
            started.elapsed().as_millis(),
            fs_response_summary(&response)
        )));
        Ok(response)
    }

    pub(crate) fn exec(&self, req: ExecRequest) -> io::Result<ExecResult> {
        let started = Instant::now();
        let timeout = Duration::from_millis(req.timeout_ms.min(15 * 60 * 1_000))
            .saturating_add(Duration::from_secs(30))
            .min(Duration::from_secs(15 * 60 + 30));
        let (mut send, mut recv) = self.node.open_stream(&self.endpoint, &self.identity)?;
        let result =
            self.node
                .block_on(io_deadline::run_for("peer exec request", timeout, async {
                    send_ctrl(&mut send, &Ctrl::Exec { req }).await?;
                    match recv_ctrl(&mut recv).await? {
                        Ctrl::ExecResp { result } => Ok(result),
                        Ctrl::ExecErr { msg } => Err(eio(msg)),
                        _ => Err(eio("Peer sendet falsche Exec-Antwort")),
                    }
                }));
        if result.is_err() {
            io_deadline::abort(&mut send, &mut recv);
        }
        let response = result?;
        let _ = self.node.ev.send(ShareEvent::Status(format!(
            "Share-Exec: {} ms, code={:?}, timeout={}",
            started.elapsed().as_millis(),
            response.exit_code,
            response.timed_out
        )));
        Ok(response)
    }
}

impl Backend for PeerBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".to_string()
    }

    fn state_identity(&self) -> String {
        let (kind, relation) = relation_kind_id(&self.endpoint);
        format!("peer:{kind}:{relation}:{}", self.endpoint.presence.node_id)
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        match self.request(FsRequest::ListDir {
            path: path.to_string(),
        })? {
            FsResponse::Entries { entries } => Ok(entries.into_iter().map(Into::into).collect()),
            _ => Err(eio("unerwartete Antwort auf list_dir")),
        }
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        match self.request(FsRequest::Stat {
            path: path.to_string(),
        })? {
            FsResponse::Meta { meta } => Ok(meta.into()),
            _ => Err(eio("unerwartete Antwort auf stat")),
        }
    }

    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        super::fs_error::exists_from_stat(self.stat(path))
    }

    fn supports_walk_tree(&self) -> bool {
        true
    }

    fn walk_tree(
        &self,
        root: &str,
        on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
    ) -> VfsResult<Option<crate::agent_proto::WireNode>> {
        super::walk::walk_peer(self, root, on_progress)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let (mut send, mut recv) = self.node.open_stream(&self.endpoint, &self.identity)?;
        let result = self
            .node
            .block_on(io_deadline::run("peer read open", async {
                send_ctrl(
                    &mut send,
                    &Ctrl::Fs {
                        req: FsRequest::Read {
                            path: path.to_string(),
                        },
                    },
                )
                .await?;
                match recv_resp(&mut recv).await? {
                    FsResponse::Data { size } => Ok(size),
                    _ => Err(eio("unerwartete Antwort auf read")),
                }
            }));
        if result.is_err() {
            io_deadline::abort(&mut send, &mut recv);
        }
        let size = result?;
        Ok(super::peer_read::reader(
            self.node.clone(),
            recv,
            size,
            TAG_DATA,
        ))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let (mut send, mut recv) = self.node.open_stream(&self.endpoint, &self.identity)?;
        self.node.block_on(async {
            send_ctrl(
                &mut send,
                &Ctrl::Fs {
                    req: FsRequest::Write {
                        path: path.to_string(),
                    },
                },
            )
            .await?;
            match recv_resp(&mut recv).await? {
                FsResponse::Ready => Ok(()),
                _ => Err(eio("unerwartete Antwort auf write")),
            }
        })?;
        Ok(super::peer_writer::writer(self.node.clone(), send, recv))
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        match self.request(FsRequest::CopyFile {
            src: src.to_string(),
            dst: dst.to_string(),
        })? {
            FsResponse::Data { size } => Ok(size),
            FsResponse::Ok => Ok(self.stat(dst).map(|metadata| metadata.size).unwrap_or(0)),
            _ => Err(eio("unerwartete Antwort auf copy_file")),
        }
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        match self.request(FsRequest::Rename {
            src: src.to_string(),
            dst: dst.to_string(),
        })? {
            FsResponse::Ok => Ok(()),
            _ => Err(eio("unerwartete Antwort auf rename")),
        }
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        match self.request(FsRequest::RenameNoReplace {
            src: src.to_string(),
            dst: dst.to_string(),
        })? {
            FsResponse::Ok => Ok(()),
            _ => Err(eio("unerwartete Antwort auf rename_no_replace")),
        }
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        match self.request(FsRequest::PromoteStaged {
            staged: staged.to_string(),
            destination: destination.to_string(),
        })? {
            FsResponse::Ok => Ok(()),
            _ => Err(eio("unerwartete Antwort auf promote_staged")),
        }
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        match self.request(FsRequest::RemoveFile {
            path: path.to_string(),
        })? {
            FsResponse::Ok => Ok(()),
            _ => Err(eio("unerwartete Antwort auf remove_file")),
        }
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        match self.request(FsRequest::RemoveDir {
            path: path.to_string(),
        })? {
            FsResponse::Ok => Ok(()),
            _ => Err(eio("unerwartete Antwort auf remove_dir")),
        }
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        match self.request(FsRequest::MkdirAll {
            path: path.to_string(),
        })? {
            FsResponse::Ok => Ok(()),
            _ => Err(eio("unerwartete Antwort auf mkdir_all")),
        }
    }

    fn parallelism(&self) -> usize {
        8
    }

    fn rename_overwrites(&self) -> bool {
        // A share can expose any backend. Safe staged replacement is delegated
        // to the resolved host backend through PromoteStaged instead.
        false
    }
}

fn fs_request_label(request: &FsRequest) -> &'static str {
    match request {
        FsRequest::ListDir { .. } => "list_dir",
        FsRequest::Stat { .. } => "stat",
        FsRequest::WalkTree { .. } => "walk_tree",
        FsRequest::Read { .. } => "read",
        FsRequest::Write { .. } => "write",
        FsRequest::WriteDone => "write_done",
        FsRequest::MkdirAll { .. } => "mkdir_all",
        FsRequest::Rename { .. } => "rename",
        FsRequest::RenameNoReplace { .. } => "rename_no_replace",
        FsRequest::PromoteStaged { .. } => "promote_staged",
        FsRequest::CopyFile { .. } => "copy_file",
        FsRequest::RemoveFile { .. } => "remove_file",
        FsRequest::RemoveDir { .. } => "remove_dir",
    }
}

fn fs_response_summary(response: &FsResponse) -> String {
    match response {
        FsResponse::Entries { entries } => format!("{} Eintraege", entries.len()),
        FsResponse::Meta { meta } => format!("meta size={} dir={}", meta.size, meta.is_dir),
        FsResponse::WalkBatch {
            nodes,
            files,
            dirs,
            bytes,
        } => format!(
            "walk nodes={} files={files} dirs={dirs} bytes={bytes}",
            nodes.len()
        ),
        FsResponse::WalkDone {
            files,
            dirs,
            bytes,
            nodes,
        } => format!("walk done nodes={nodes} files={files} dirs={dirs} bytes={bytes}"),
        FsResponse::Data { size } => format!("{size} bytes"),
        FsResponse::Ready => "bereit".into(),
        FsResponse::Ok => "ok".into(),
        FsResponse::Err { msg, .. } => format!("fehler={msg}"),
    }
}

impl From<FsMeta> for VfsMeta {
    fn from(metadata: FsMeta) -> Self {
        VfsMeta {
            name: metadata.name,
            is_dir: metadata.is_dir,
            is_symlink: metadata.is_symlink,
            size: metadata.size,
            mtime_ms: metadata.mtime_ms,
            btime_ms: metadata.btime_ms,
            hidden: metadata.hidden,
            system: metadata.system,
            id: metadata.id,
            content_md5: None,
        }
    }
}
