//! `AgentBackend` — a `vfs::Backend` that drives a remote `se-agent` over the
//! multiplexed, streaming protocol-v2 framed stdio stream (an SSH `exec` channel
//! in production, or a spawned local child / socket pair for testing). Directory
//! listing, `stat`, the whole-tree storage-analysis walk, byte transfers
//! (`open_read`/`open_write`), server-local mutations (copy/rename/remove/mkdir),
//! recursive bulk tree transfer, server-side search and the sync signature walk
//! all run SERVER-SIDE in the agent; anything unsupported (or any transport
//! error) falls back per-op to the `inner` backend (the SFTP backend in
//! production). See `docs/SSH_AGENT_PLAN.md`.
//!
//! ## Bridge worker (protocol v2)
//!
//! One channel carries every operation, tagged by `req_id`:
//!  * a **writer thread** owns the write half and serialises outgoing frames
//!    (fed by a crossbeam channel — clones of the sender live in the backend and
//!    in each active read/write stream),
//!  * a **reader thread** owns the read half, decodes frames and routes each to
//!    the waiting op's channel by `req_id`.
//! Dropping the backend (and all its streams) drops every sender → the writer
//! exits and closes the write half → the agent sees EOF on stdin and exits →
//! the reader sees EOF and exits. No explicit shutdown handshake needed.

use crate::agent_proto::{self, Frame, WireMeta};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Bound on un-sent outgoing frames. Provides backpressure for uploads (a slow
/// link makes `send` block instead of buffering the whole local tree in memory)
/// while still pipelining ~8 MiB of 256 KiB chunks ahead of the wire.
const OUT_BACKLOG: usize = 32;

/// Shared multiplexer over one agent channel.
struct Mux {
    /// Outgoing frames → the writer thread (FIFO preserves per-op ordering).
    out: Sender<(u64, Frame)>,
    /// req_id → the op waiting for its reply/stream frames.
    pending: Arc<Mutex<HashMap<u64, Sender<Frame>>>>,
    next_id: AtomicU64,
}

impl Mux {
    /// Allocate a fresh req_id and a channel to receive its frames.
    fn register(&self) -> (u64, Receiver<Frame>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = unbounded();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(id, tx);
        }
        (id, rx)
    }
    fn unregister(&self, id: u64) {
        if let Ok(mut p) = self.pending.lock() {
            p.remove(&id);
        }
    }
    fn send(&self, id: u64, frame: Frame) -> io::Result<()> {
        self.out
            .send((id, frame))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "agent writer gone"))
    }
    /// One request → one response frame (single-shot ops). Registers, sends,
    /// waits for the first frame, then unregisters.
    fn call(&self, req: Frame) -> io::Result<Frame> {
        let (id, rx) = self.register();
        let r = (|| {
            self.send(id, req)?;
            rx.recv()
                .map_err(|_| io::Error::new(io::ErrorKind::UnexpectedEof, "agent stream closed"))
        })();
        self.unregister(id);
        r
    }
}

pub struct AgentBackend {
    inner: BackendHandle,
    mux: Arc<Mux>,
    version: String,
}

impl AgentBackend {
    /// Hand-shake over an already-open framed stream pair (e.g. an SSH channel
    /// split into read/write halves).
    pub fn from_streams(
        r: Box<dyn Read + Send>,
        w: Box<dyn Write + Send>,
        inner: BackendHandle,
    ) -> io::Result<Self> {
        let (out_tx, out_rx) = bounded::<(u64, Frame)>(OUT_BACKLOG);
        let pending: Arc<Mutex<HashMap<u64, Sender<Frame>>>> = Arc::new(Mutex::new(HashMap::new()));

        // Writer thread: drain outgoing frames; closing the write half on exit
        // signals EOF to the agent.
        std::thread::Builder::new()
            .name("agent-writer".into())
            .spawn(move || {
                let mut w = w;
                while let Ok((id, frame)) = out_rx.recv() {
                    if agent_proto::write_frame(&mut w, id, &frame).is_err() {
                        break;
                    }
                }
                // w dropped here → underlying channel write side closes.
            })
            .ok();

        // Reader thread: route inbound frames to the waiting op by req_id.
        let pending_r = pending.clone();
        std::thread::Builder::new()
            .name("agent-reader".into())
            .spawn(move || {
                let mut r = r;
                loop {
                    match agent_proto::read_frame(&mut r) {
                        Ok(Some((id, frame))) => {
                            let tx = pending_r.lock().ok().and_then(|p| p.get(&id).cloned());
                            if let Some(tx) = tx {
                                let _ = tx.send(frame);
                            }
                        }
                        _ => break, // EOF or decode error
                    }
                }
                // Drop all waiters so any blocked recv() errors out → ops fall back.
                if let Ok(mut p) = pending_r.lock() {
                    p.clear();
                }
            })
            .ok();

        let mux = Arc::new(Mux { out: out_tx, pending, next_id: AtomicU64::new(1) });

        // Handshake before publishing the backend.
        let version = match mux.call(Frame::Hello { proto: agent_proto::PROTO_VERSION })? {
            Frame::HelloOk { proto, version } if proto == agent_proto::PROTO_VERSION => version,
            Frame::HelloOk { proto, .. } => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("agent protocol {proto} != {}", agent_proto::PROTO_VERSION),
                ))
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected handshake reply: {other:?}"),
                ))
            }
        };
        Ok(AgentBackend { inner, mux, version })
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

fn wire_to_vfs(m: WireMeta) -> VfsMeta {
    VfsMeta {
        name: m.name,
        is_dir: m.is_dir,
        is_symlink: m.is_symlink,
        size: m.size,
        mtime_ms: m.mtime_ms,
        btime_ms: 0,
        hidden: false,
        system: false,
        id: None,
        content_md5: None,
    }
}

/// `std::io::Read` over a streamed `Read` op: pulls `Data` frames from the mux,
/// ends on `End`, fails on `Err`/transport drop. Cancels the op if dropped early.
struct AgentReadStream {
    mux: Arc<Mux>,
    id: u64,
    rx: Receiver<Frame>,
    buf: Vec<u8>,
    pos: usize,
    done: bool,
}

impl Read for AgentReadStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.pos < self.buf.len() {
                let n = (self.buf.len() - self.pos).min(out.len());
                out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            if self.done {
                return Ok(0);
            }
            match self.rx.recv() {
                Ok(Frame::Data(d)) => {
                    self.buf = d;
                    self.pos = 0;
                }
                Ok(Frame::End) => {
                    self.done = true;
                    return Ok(0);
                }
                Ok(Frame::Err(e)) => return Err(io::Error::other(e)),
                Ok(_) => continue, // ignore unexpected frame kinds
                Err(_) => {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "agent read stream closed"))
                }
            }
        }
    }
}

impl Drop for AgentReadStream {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.mux.send(self.id, Frame::Cancel);
        }
        self.mux.unregister(self.id);
    }
}

/// `std::io::Write` over a streamed `Write` op: each `write` ships a `Data`
/// frame; closing sends `End` and waits for the agent's `Ok`/`Err` (the server
/// writes to a temp, fsyncs and atomically renames). Mirrors `SftpWriter`'s
/// close-on-drop semantics.
struct AgentWriteStream {
    mux: Arc<Mux>,
    id: u64,
    rx: Receiver<Frame>,
    finished: bool,
}

impl AgentWriteStream {
    fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.mux.send(self.id, Frame::End)?;
        let r = match self.rx.recv() {
            Ok(Frame::Ok) => Ok(()),
            Ok(Frame::Err(e)) => Err(io::Error::other(e)),
            Ok(_) => Err(io::Error::other("unexpected agent reply to write")),
            Err(_) => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "agent write stream closed")),
        };
        self.mux.unregister(self.id);
        r
    }
}

impl Write for AgentWriteStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.mux.send(self.id, Frame::Data(buf.to_vec()))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(()) // the agent fsyncs at End; chunks are flushed on the wire as sent
    }
}

impl Drop for AgentWriteStream {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

impl AgentBackend {
    /// Begin a streamed read of `path`. Blocks for the first frame so an open
    /// error falls back to `inner` synchronously (the feature only ever speeds
    /// up — it never breaks a read).
    fn agent_open_read(&self, path: &str) -> Option<Box<dyn Read + Send>> {
        let (id, rx) = self.mux.register();
        if self
            .mux
            .send(id, Frame::Read { path: path.to_string(), offset: 0, len: 0 })
            .is_err()
        {
            self.mux.unregister(id);
            return None;
        }
        match rx.recv() {
            Ok(Frame::Data(d)) => Some(Box::new(AgentReadStream {
                mux: self.mux.clone(),
                id,
                rx,
                buf: d,
                pos: 0,
                done: false,
            })),
            Ok(Frame::End) => Some(Box::new(AgentReadStream {
                mux: self.mux.clone(),
                id,
                rx,
                buf: Vec::new(),
                pos: 0,
                done: true,
            })),
            // Err or unexpected → fall back to the inner backend.
            _ => {
                self.mux.unregister(id);
                None
            }
        }
    }

    /// Begin a streamed write of `path`. Blocks for the agent's ready-ack so a
    /// path/permission error falls back to `inner` synchronously (parity with
    /// SFTP's fail-fast `open_write`).
    fn agent_open_write(&self, path: &str) -> Option<Box<dyn Write + Send>> {
        let (id, rx) = self.mux.register();
        if self.mux.send(id, Frame::Write(path.to_string())).is_err() {
            self.mux.unregister(id);
            return None;
        }
        match rx.recv() {
            // Progress{0,0} = "temp created, ready to receive".
            Ok(Frame::Progress { .. }) => {
                Some(Box::new(AgentWriteStream { mux: self.mux.clone(), id, rx, finished: false }))
            }
            _ => {
                self.mux.unregister(id);
                None
            }
        }
    }

    /// Run a single-shot mutation op that replies `Ok`/`Err`. `Ok(true)` ran on
    /// the agent, `Ok(false)` means "fall back to inner", `Err` is a real error
    /// the agent reported (don't paper over it with a fallback that would also
    /// fail — e.g. "directory not empty").
    fn agent_unit_op(&self, req: Frame) -> io::Result<bool> {
        match self.mux.call(req) {
            Ok(Frame::Ok) => Ok(true),
            Ok(Frame::Err(e)) => Err(io::Error::other(e)),
            _ => Ok(false), // transport/handshake oddity → caller falls back
        }
    }

    /// Stream an entire remote subtree (`root`) down into local `dst` in one
    /// session — the contents of `root` land directly under `dst`.
    fn agent_get_tree(&self, root: &str, dst: &Path) -> io::Result<u64> {
        std::fs::create_dir_all(dst)?;
        let (id, rx) = self.mux.register();
        let r = (|| {
            self.mux.send(id, Frame::GetTree(root.to_string()))?;
            let mut cur: Option<std::fs::File> = None;
            let mut files = 0u64;
            loop {
                match rx.recv() {
                    Ok(Frame::TreeEntry { rel, is_dir, .. }) => {
                        cur = None;
                        let p = dst.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
                        if is_dir {
                            std::fs::create_dir_all(&p)?;
                        } else {
                            if let Some(parent) = p.parent() {
                                std::fs::create_dir_all(parent)?;
                            }
                            cur = Some(std::fs::File::create(&p)?);
                            files += 1;
                        }
                    }
                    Ok(Frame::Data(d)) => {
                        if let Some(f) = cur.as_mut() {
                            f.write_all(&d)?;
                        }
                    }
                    Ok(Frame::End) => break,
                    Ok(Frame::Err(e)) => return Err(io::Error::other(e)),
                    Ok(_) => {}
                    Err(_) => {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "agent get-tree closed"))
                    }
                }
            }
            Ok(files)
        })();
        self.mux.unregister(id);
        r
    }

    /// Stream an entire local subtree (`src`) up into remote `root` in one
    /// session — the contents of `src` land directly under `root`.
    fn agent_put_tree(&self, src: &Path, root: &str) -> io::Result<u64> {
        let (id, rx) = self.mux.register();
        let r = (|| {
            self.mux.send(id, Frame::PutTree(root.to_string()))?;
            let mut files = 0u64;
            send_subtree(&self.mux, id, src, src, &mut files)?;
            self.mux.send(id, Frame::End)?;
            match rx.recv() {
                Ok(Frame::Ok) => Ok(files),
                Ok(Frame::Err(e)) => Err(io::Error::other(e)),
                _ => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "agent put-tree closed")),
            }
        })();
        self.mux.unregister(id);
        r
    }
}

/// Depth-first walk of a local subtree, emitting `TreeEntry` (rel path with `/`
/// separators, relative to `base`) and, for files, a run of `Data` chunks.
/// `mux.send` blocks on the bounded outgoing channel → natural upload pacing.
fn send_subtree(mux: &Mux, id: u64, base: &Path, dir: &Path, files: &mut u64) -> io::Result<()> {
    for ent in std::fs::read_dir(dir)? {
        let ent = ent?;
        let ft = match ent.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_symlink() {
            continue;
        }
        let p = ent.path();
        let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/");
        if ft.is_dir() {
            mux.send(id, Frame::TreeEntry { rel, is_dir: true, size: 0, mtime_ms: 0 })?;
            send_subtree(mux, id, base, &p, files)?;
        } else if ft.is_file() {
            let md = ent.metadata().ok();
            let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
            mux.send(id, Frame::TreeEntry { rel, is_dir: false, size, mtime_ms: 0 })?;
            let mut f = std::fs::File::open(&p)?;
            let mut buf = vec![0u8; agent_proto::CHUNK];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                mux.send(id, Frame::Data(buf[..n].to_vec()))?;
            }
            *files += 1;
        }
    }
    Ok(())
}

impl Backend for AgentBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        match self.mux.call(Frame::ListDir(path.to_string())) {
            Ok(Frame::Dir(v)) => Ok(v.into_iter().map(wire_to_vfs).collect()),
            Ok(Frame::Err(e)) => Err(io::Error::other(e)),
            // Unexpected reply or transport failure → fall back so browsing
            // degrades to plain SFTP rather than breaking.
            _ => self.inner.list_dir(path),
        }
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        match self.mux.call(Frame::Stat(path.to_string())) {
            Ok(Frame::Meta(m)) => Ok(wire_to_vfs(m)),
            Ok(Frame::Err(e)) => Err(io::Error::other(e)),
            _ => self.inner.stat(path),
        }
    }

    fn supports_walk_tree(&self) -> bool {
        true
    }

    fn walk_tree(&self, root: &str) -> Option<crate::agent_proto::WireNode> {
        match self.mux.call(Frame::WalkTree(root.to_string())) {
            Ok(Frame::Tree(n)) => Some(n),
            _ => None, // fall back to the client-side walk
        }
    }

    // ── Phase 4: recursive bulk folder transfer in one session ──
    fn supports_bulk_tree(&self) -> bool {
        true
    }
    fn get_tree(&self, root: &str, dst: &Path) -> VfsResult<u64> {
        self.agent_get_tree(root, dst)
    }
    fn put_tree(&self, src: &Path, root: &str) -> VfsResult<u64> {
        self.agent_put_tree(src, root)
    }

    // ── Phase 1: streamed read via the agent, SFTP fallback ──
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        match self.agent_open_read(path) {
            Some(r) => Ok(r),
            None => self.inner.open_read(path),
        }
    }
    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        // Agent indexes by path; the id is only meaningful to id-keyed backends
        // (Google Drive), which never sit behind the agent.
        let _ = id;
        self.open_read(path)
    }

    // ── Phase 2: streamed write via the agent, SFTP fallback ──
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        match self.agent_open_write(path) {
            Some(w) => Ok(w),
            None => self.inner.open_write(path),
        }
    }
    fn download_name(&self, path: &str, name: &str) -> String {
        self.inner.download_name(path, name)
    }

    // ── Phase 3: server-local mutations via the agent, SFTP fallback ──
    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        // Server-local copy is the big win (no down+up through a temp). The agent
        // reports success only, so report the destination size for progress.
        match self.agent_unit_op(Frame::Copy { src: src.to_string(), dst: dst.to_string() }) {
            Ok(true) => Ok(self.stat(dst).map(|m| m.size).unwrap_or(0)),
            Ok(false) => self.inner.copy_file(src, dst),
            Err(e) => Err(e),
        }
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        match self.agent_unit_op(Frame::Rename { src: src.to_string(), dst: dst.to_string() }) {
            Ok(true) => Ok(()),
            Ok(false) => self.inner.rename(src, dst),
            Err(e) => Err(e),
        }
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        match self.agent_unit_op(Frame::Remove { path: path.to_string(), recursive: false }) {
            Ok(true) => Ok(()),
            Ok(false) => self.inner.remove_file(path),
            Err(e) => Err(e),
        }
    }
    fn remove_file_id(&self, path: &str, _id: Option<&str>) -> VfsResult<()> {
        // Agent indexes by path; id-keyed backends (Drive) never sit behind it.
        self.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        match self.agent_unit_op(Frame::Remove { path: path.to_string(), recursive: false }) {
            Ok(true) => Ok(()),
            Ok(false) => self.inner.remove_dir(path),
            Err(e) => Err(e),
        }
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        match self.agent_unit_op(Frame::Mkdir(path.to_string())) {
            Ok(true) => Ok(()),
            Ok(false) => self.inner.mkdir_all(path),
            Err(e) => Err(e),
        }
    }
    fn parallelism(&self) -> usize {
        self.inner.parallelism()
    }
    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }
    fn is_local(&self) -> bool {
        self.inner.is_local()
    }
    fn provides_content_hash(&self) -> bool {
        self.inner.provides_content_hash()
    }
}

// ── deploy over an existing SFTP/SSH connection (phase 3) ────────────────────

/// A bundled agent binary for one server target. The integrity hash is computed
/// from `bytes` at deploy time (sha2) — no separate hash to keep in sync.
pub struct AgentArtifact {
    pub bytes: &'static [u8],
}

/// Select the bundled agent for a server's `uname -sm` (e.g. "Linux x86_64",
/// "Linux aarch64"), or `None` if we ship none for it → the caller keeps plain
/// SFTP. Binaries are static-musl, built by the standalone `se-agent` crate
/// (see `docs/SSH_AGENT_PLAN.md` §7) and embedded here.
pub fn artifact_for(uname_sm: &str) -> Option<AgentArtifact> {
    let mut it = uname_sm.split_whitespace();
    let os = it.next().unwrap_or("");
    let arch = it.next().unwrap_or("");
    let bytes: &'static [u8] = match (os, arch) {
        ("Linux", "x86_64") => include_bytes!("../agent-bin/se-agent-x86_64-linux-musl"),
        ("Linux", "aarch64") | ("Linux", "arm64") => {
            include_bytes!("../agent-bin/se-agent-aarch64-linux-musl")
        }
        _ => return None,
    };
    Some(AgentArtifact { bytes })
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{:02x}", b)).collect()
}

/// Single-quote a string for safe interpolation into a remote `sh -c` command.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r#"'\''"#))
}

/// Deploy + launch the agent over an existing SFTP backend's SSH connection:
/// detect the target, upload (verified) if missing/stale, then exec `--serve`
/// and hand-shake. Returns an `AgentBackend` wrapping `inner` (the SFTP backend)
/// for byte-stream ops. Any failure is returned so the caller falls back to
/// plain SFTP. See `docs/SSH_AGENT_PLAN.md` §5/§6.
pub fn deploy_over_sftp(
    sftp: &crate::sftp::SftpBackend,
    inner: BackendHandle,
) -> io::Result<AgentBackend> {
    // 1. Detect the server target and pick a matching bundled binary.
    let uname = sftp.exec_capture("uname -sm")?;
    let art = artifact_for(&uname)
        .ok_or_else(|| io::Error::other(format!("kein Agent-Binary gebündelt für '{uname}'")))?;

    // 2. Resolve a per-user install path, keyed by PROTO version (so a stale but
    //    protocol-compatible agent is reused, and app-version bumps don't churn).
    let home = sftp.exec_capture("printf %s \"$HOME\"")?;
    let home = if home.is_empty() { ".".to_string() } else { home };
    let dir = format!("{}/.cache/smart-explorer", home.trim_end_matches('/'));
    let remote = format!("{}/se-agent-p{}", dir, agent_proto::PROTO_VERSION);

    // 3. Skip upload if a protocol-compatible agent is already installed.
    let want = format!("proto={}", agent_proto::PROTO_VERSION);
    let probe = sftp
        .exec_capture(&format!("{} --version 2>/dev/null", sh_quote(&remote)))
        .unwrap_or_default();
    if !probe.contains(&want) {
        // 4. Upload to a temp path (SFTP), verify SHA-256 server-side against the
        //    bundled bytes, then atomically move into place and lock perms.
        inner.mkdir_all(&dir)?;
        let tmp = format!("{}.tmp", remote);
        {
            let mut w = inner.open_write(&tmp)?;
            w.write_all(art.bytes)?;
            w.flush()?;
        }
        let expected = sha256_hex(art.bytes);
        let sum = sftp
            .exec_capture(&format!("sha256sum {} 2>/dev/null | cut -d' ' -f1", sh_quote(&tmp)))
            .unwrap_or_default();
        if !sum.is_empty() && !sum.eq_ignore_ascii_case(&expected) {
            let _ = inner.remove_file(&tmp);
            return Err(io::Error::other("Agent-Binary: SHA-256 stimmt nicht"));
        }
        sftp.exec_capture(&format!(
            "mv -f {tmp} {remote} && chmod 700 {remote}",
            tmp = sh_quote(&tmp),
            remote = sh_quote(&remote),
        ))?;
    }

    // 5. Launch the agent and hand-shake.
    let (r, w) = sftp.open_exec_streams(&format!("{} --serve", sh_quote(&remote)))?;
    AgentBackend::from_streams(r, w, inner)
}

/// Remove a deployed agent from a server (the "Remote-Agent entfernen" action).
pub fn remove_from_sftp(sftp: &crate::sftp::SftpBackend) -> io::Result<()> {
    sftp.exec_capture("rm -rf \"$HOME/.cache/smart-explorer\"")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn artifact_selection_and_quoting() {
        let a = artifact_for("Linux x86_64").expect("x86_64 bundled");
        assert!(a.bytes.len() > 1000 && a.bytes.starts_with(b"\x7fELF"));
        assert!(artifact_for("Linux aarch64").is_some());
        assert!(artifact_for("Darwin arm64").is_none());
        assert!(artifact_for("garbage").is_none());
        assert_eq!(sha256_hex(a.bytes).len(), 64);
        assert_eq!(sh_quote("/home/u/dir"), "'/home/u/dir'");
        assert_eq!(sh_quote("a'b; rm -rf /"), r#"'a'\''b; rm -rf /'"#);
    }

    /// Spawn an in-process agent (`agent_proto::serve`) on one end of a TCP
    /// socket pair and drive a real `AgentBackend` from the other — exercises
    /// the full v2 mux (handshake, list, walk, stat, streamed read) with no
    /// child process / SSH.
    #[test]
    fn agent_backend_over_socket() {
        let base = std::env::temp_dir().join(format!("se_agbe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("a.txt"), vec![7u8; 100]).unwrap();
        std::fs::write(base.join("sub/b.bin"), vec![0u8; 400]).unwrap();
        let root = base.to_string_lossy().to_string();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let r = sock.try_clone().unwrap();
            let _ = agent_proto::serve(r, sock);
        });

        let client = TcpStream::connect(addr).unwrap();
        // A spare handle to force a FIN at the end: a TCP socket only sends FIN
        // once ALL clones close, and the bridge's reader thread keeps one clone
        // blocked on read — so we shut it down explicitly (the SSH/child-process
        // transports EOF naturally when the single write end drops).
        let shut = client.try_clone().unwrap();
        let r: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
        let w: Box<dyn Write + Send> = Box::new(client);
        let inner: BackendHandle = std::sync::Arc::new(crate::vfs::LocalBackend::new("/"));
        let be = AgentBackend::from_streams(r, w, inner).unwrap();

        // list_dir
        let mut entries = be.list_dir(&root).unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries.iter().find(|e| e.name == "a.txt").unwrap().size, 100);
        assert!(entries.iter().find(|e| e.name == "sub").unwrap().is_dir);

        // whole-tree walk
        assert!(be.supports_walk_tree());
        let tree = crate::analytics::from_wire(be.walk_tree(&root).unwrap());
        assert_eq!(tree.size, 500);
        assert_eq!(tree.children.iter().find(|c| &*c.name == "sub").unwrap().size, 400);

        // stat
        let m = be.stat(&format!("{}/a.txt", root)).unwrap();
        assert_eq!(m.size, 100);
        assert!(!m.is_dir);

        // streamed read (Phase 1)
        let mut buf = Vec::new();
        be.open_read(&format!("{}/a.txt", root)).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, vec![7u8; 100]);

        // streamed write (Phase 2): temp + atomic rename, server-side
        {
            let mut w = be.open_write(&format!("{}/written.dat", root)).unwrap();
            w.write_all(b"hello agent write").unwrap();
            w.flush().unwrap();
        } // drop → End + Ok ack
        assert_eq!(std::fs::read(base.join("written.dat")).unwrap(), b"hello agent write");

        // server-local mutations (Phase 3): mkdir, copy, rename, remove
        be.mkdir_all(&format!("{}/newdir/inner", root)).unwrap();
        assert!(base.join("newdir/inner").is_dir());
        be.copy_file(&format!("{}/a.txt", root), &format!("{}/newdir/copy.txt", root)).unwrap();
        assert_eq!(std::fs::read(base.join("newdir/copy.txt")).unwrap().len(), 100);
        be.rename(&format!("{}/newdir/copy.txt", root), &format!("{}/newdir/moved.txt", root)).unwrap();
        assert!(!base.join("newdir/copy.txt").exists() && base.join("newdir/moved.txt").exists());
        be.remove_file(&format!("{}/newdir/moved.txt", root)).unwrap();
        assert!(!base.join("newdir/moved.txt").exists());
        // a real error (removing a non-empty dir non-recursively) surfaces, not swallowed
        assert!(be.remove_dir(&format!("{}/newdir", root)).is_err());

        // recursive bulk transfer (Phase 4): put a local tree up, get it back
        assert!(be.supports_bulk_tree());
        let upsrc = base.join("upsrc");
        std::fs::create_dir_all(upsrc.join("sub")).unwrap();
        std::fs::write(upsrc.join("f1.txt"), b"one").unwrap();
        std::fs::write(upsrc.join("sub/f2.txt"), b"two longer").unwrap();
        let remote_dst = format!("{}/uploaded", root);
        assert_eq!(be.put_tree(&upsrc, &remote_dst).unwrap(), 2);
        assert_eq!(std::fs::read(base.join("uploaded/f1.txt")).unwrap(), b"one");
        assert_eq!(std::fs::read(base.join("uploaded/sub/f2.txt")).unwrap(), b"two longer");
        let getdst = base.join("downloaded");
        assert_eq!(be.get_tree(&remote_dst, &getdst).unwrap(), 2);
        assert_eq!(std::fs::read(getdst.join("f1.txt")).unwrap(), b"one");
        assert_eq!(std::fs::read(getdst.join("sub/f2.txt")).unwrap(), b"two longer");

        drop(be);
        let _ = shut.shutdown(std::net::Shutdown::Both);
        let _ = server.join();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Drive the ACTUAL bundled musl binary as a child process over its stdio —
    /// the real deployable artifact, end to end (handshake + list + streamed
    /// read). Only meaningful where the host can run the x86_64 binary.
    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn real_agent_binary_child_process() {
        use std::process::{Command, Stdio};
        let bin = concat!(env!("CARGO_MANIFEST_DIR"), "/agent-bin/se-agent-x86_64-linux-musl");
        let base = std::env::temp_dir().join(format!("se_agbin_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("d")).unwrap();
        std::fs::write(base.join("hello.txt"), b"agent works").unwrap();
        std::fs::write(base.join("d/x.bin"), vec![9u8; 300]).unwrap();
        let root = base.to_string_lossy().to_string();

        let mut child = match Command::new(bin)
            .arg("--serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return, // can't exec the bundled binary here → skip
        };
        let w: Box<dyn Write + Send> = Box::new(child.stdin.take().unwrap());
        let r: Box<dyn Read + Send> = Box::new(child.stdout.take().unwrap());
        let inner: BackendHandle = std::sync::Arc::new(crate::vfs::LocalBackend::new("/"));
        let be = AgentBackend::from_streams(r, w, inner).unwrap();
        assert!(be.version().contains('.'));

        let mut entries = be.list_dir(&root).unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(entries.len(), 2);

        let tree = crate::analytics::from_wire(be.walk_tree(&root).unwrap());
        assert_eq!(tree.size, 311);

        let mut buf = String::new();
        be.open_read(&format!("{}/hello.txt", root)).unwrap().read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "agent works");

        // write + bulk tree against the REAL bundled binary
        {
            let mut w = be.open_write(&format!("{}/up.txt", root)).unwrap();
            w.write_all(b"streamed up").unwrap();
            w.flush().unwrap();
        }
        assert_eq!(std::fs::read(base.join("up.txt")).unwrap(), b"streamed up");
        // Download target OUTSIDE the walked root (in production the agent walks
        // the remote fs and the client writes locally; here both are one fs).
        let getdst = std::env::temp_dir().join(format!("se_got_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&getdst);
        assert_eq!(be.get_tree(&root, &getdst).unwrap(), 3); // hello.txt, d/x.bin, up.txt
        assert!(getdst.join("d/x.bin").exists());

        drop(be);
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&getdst);
    }
}
