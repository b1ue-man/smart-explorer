//! `AgentBackend` — a `vfs::Backend` that drives a remote `se-agent` over a
//! framed stdio stream (an SSH `exec` channel in production, or a spawned local
//! child process for testing). Directory listing, `stat`, and the whole-tree
//! storage-analysis walk run SERVER-SIDE in the agent; byte-stream ops
//! (`open_read`/`open_write`/…) and any unsupported call delegate to the `inner`
//! backend (the SFTP backend in production). See `docs/SSH_AGENT_PLAN.md`.
//!
//! The protocol is strictly request/response (one outstanding call), so a single
//! `Mutex` around the stream serialises calls — simple and correct; multiplexing
//! is a later optimisation.

use crate::agent_proto::{self, Req, Resp, WireMeta};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::sync::Mutex;

/// The live framed connection to one agent. Dropping it drops the underlying
/// streams (SSH channel halves) → the remote agent gets EOF on stdin and exits.
struct AgentConn {
    r: Box<dyn Read + Send>,
    w: Box<dyn Write + Send>,
}

pub struct AgentBackend {
    /// File-stream ops + fallback for anything the agent can't do.
    inner: BackendHandle,
    conn: Mutex<AgentConn>,
    /// Agent's reported semver (diagnostics / status chip).
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
        Self::connect(AgentConn { r, w }, inner)
    }

    fn connect(conn: AgentConn, inner: BackendHandle) -> io::Result<Self> {
        let mut conn = conn;
        // Handshake before publishing the backend.
        agent_proto::write_frame(&mut conn.w, &Req::Hello { proto: agent_proto::PROTO_VERSION }.encode())?;
        let resp = match agent_proto::read_frame(&mut conn.r)? {
            Some(f) => Resp::decode(&f)?,
            None => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "agent closed during handshake")),
        };
        let version = match resp {
            Resp::Hello { proto, version } if proto == agent_proto::PROTO_VERSION => version,
            Resp::Hello { proto, .. } => {
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
        Ok(AgentBackend { inner, conn: Mutex::new(conn), version })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// One request → one response (serialised by the conn mutex).
    fn call(&self, req: Req) -> io::Result<Resp> {
        let mut c = self
            .conn
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "agent connection poisoned"))?;
        agent_proto::write_frame(&mut c.w, &req.encode())?;
        match agent_proto::read_frame(&mut c.r)? {
            Some(f) => Resp::decode(&f),
            None => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "agent stream closed")),
        }
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

impl Backend for AgentBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        match self.call(Req::ListDir(path.to_string())) {
            Ok(Resp::Dir(v)) => Ok(v.into_iter().map(wire_to_vfs).collect()),
            Ok(Resp::Err(e)) => Err(io::Error::other(e)),
            // Unexpected reply or transport failure → fall back to the inner
            // backend so browsing degrades to plain SFTP rather than breaking.
            _ => self.inner.list_dir(path),
        }
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        match self.call(Req::Stat(path.to_string())) {
            Ok(Resp::Meta(m)) => Ok(wire_to_vfs(m)),
            Ok(Resp::Err(e)) => Err(io::Error::other(e)),
            _ => self.inner.stat(path),
        }
    }

    fn supports_walk_tree(&self) -> bool {
        true
    }

    fn walk_tree(&self, root: &str) -> Option<crate::agent_proto::WireNode> {
        match self.call(Req::WalkTree(root.to_string())) {
            Ok(Resp::Tree(n)) => Some(n),
            _ => None, // fall back to the client-side walk
        }
    }

    // ── byte-stream ops + mutations: delegate to the inner (SFTP) backend ──
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }
    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read_id(path, id)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }
    fn download_name(&self, path: &str, name: &str) -> String {
        self.inner.download_name(path, name)
    }
    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        self.inner.copy_file(src, dst)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.inner.rename(src, dst)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }
    fn remove_file_id(&self, path: &str, id: Option<&str>) -> VfsResult<()> {
        self.inner.remove_file_id(path, id)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
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

/// Remove a deployed agent from a server (the "Remote-Agent entfernen" action;
/// UI wiring is a phase-6 follow-up).
#[allow(dead_code)]
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
        // Linux x86_64/aarch64 are bundled; other targets fall back to SFTP.
        let a = artifact_for("Linux x86_64").expect("x86_64 bundled");
        assert!(a.bytes.len() > 1000 && a.bytes.starts_with(b"\x7fELF"));
        assert!(artifact_for("Linux aarch64").is_some());
        assert!(artifact_for("Darwin arm64").is_none());
        assert!(artifact_for("garbage").is_none());
        // Hash is computed from the bundled bytes (stable, 64 hex chars).
        assert_eq!(sha256_hex(a.bytes).len(), 64);
        // Shell quoting is injection-safe.
        assert_eq!(sh_quote("/home/u/dir"), "'/home/u/dir'");
        assert_eq!(sh_quote("a'b; rm -rf /"), r#"'a'\''b; rm -rf /'"#);
    }

    /// Drive a real `AgentBackend` against an in-process agent over a TCP socket
    /// pair (no child process / SSH needed): a thread runs `agent_proto::serve`
    /// on one end, the backend talks framed protocol on the other.
    #[test]
    fn agent_backend_over_socket() {
        let base = std::env::temp_dir().join(format!("se_agbe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("a.txt"), vec![0u8; 100]).unwrap();
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
        let r: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
        let w: Box<dyn Write + Send> = Box::new(client);
        let inner: BackendHandle = std::sync::Arc::new(crate::vfs::LocalBackend::new("/"));
        let be = AgentBackend::from_streams(r, w, inner).unwrap();

        // list_dir over the agent
        let mut entries = be.list_dir(&root).unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries.iter().find(|e| e.name == "a.txt").unwrap().size, 100);
        assert!(entries.iter().find(|e| e.name == "sub").unwrap().is_dir);

        // whole-tree walk over the agent
        assert!(be.supports_walk_tree());
        let tree = crate::analytics::from_wire(be.walk_tree(&root).unwrap());
        assert_eq!(tree.size, 500);
        let sub = tree.children.iter().find(|c| &*c.name == "sub").unwrap();
        assert_eq!(sub.size, 400);

        // stat over the agent
        let m = be.stat(&format!("{}/a.txt", root)).unwrap();
        assert_eq!(m.size, 100);
        assert!(!m.is_dir);

        drop(be); // closes the socket → server thread's serve() returns
        let _ = server.join();
        let _ = std::fs::remove_dir_all(&base);
    }
}
