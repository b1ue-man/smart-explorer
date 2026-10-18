//! Backends of the FUSE drive signals (DRV-M4), each behind the daemon's
//! authorized root with the POSIX name rules (`RootedBackend::new_in`), as
//! the daemon serves a Linux drive: a local folder, the deterministic
//! in-memory fixture, the SFTP container of the task stage (`SE_TASK_SFTP_*`,
//! fixture `sftp`), a read-only tree with names Windows cannot hold plus a
//! link and a FIFO, and the in-memory fixture with a gate that holds one
//! content download or upload until the test releases it.
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use crate::mount::optimization_fixture::OptimizationBackend;
use crate::mount::{BackendRoot, MountId, MountMode, MountRootSecurity, PathDialect};
use crate::vfs::{
    Backend, BackendHandle, LocalBackend, RootConfinement, Scheme, StagedWriteCapabilities,
    VfsMeta, VfsResult,
};

/// `raw` below `root`, as the daemon serves it to a Linux drive host.
pub(super) fn rooted(raw: BackendHandle, root: &str, mode: MountMode) -> io::Result<BackendHandle> {
    super::super::rooted_backend::RootedBackend::new_in(
        raw,
        &BackendRoot::parse(root)?,
        mode,
        MountRootSecurity::Trusted,
        PathDialect::Posix,
    )
}

/// A local folder as the drive.
pub(super) fn local(folder: &Path, mode: MountMode) -> io::Result<BackendHandle> {
    let root = folder
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "fixture path"))?;
    rooted(Arc::new(LocalBackend::new(root)), root, mode)
}

/// The in-memory fixture as the drive.
pub(super) fn memory(mode: MountMode) -> io::Result<(Arc<OptimizationBackend>, BackendHandle)> {
    let fixture = OptimizationBackend::new();
    let drive = rooted(fixture.clone(), "/", mode)?;
    Ok((fixture, drive))
}

/// A fresh folder in the SFTP container of the task stage as the drive;
/// returns the raw backend, the drive and the folder on the server.
pub(super) fn sftp(mode: MountMode) -> io::Result<(BackendHandle, BackendHandle, String)> {
    let host = variable("SE_TASK_SFTP_HOST")?;
    let port: u16 = variable("SE_TASK_SFTP_PORT")?
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "SE_TASK_SFTP_PORT"))?;
    trust_scanned_host_key(&host, port)?;
    let base = variable("SE_TASK_SFTP_ROOT")?;
    let raw: BackendHandle = Arc::new(crate::sftp::SftpBackend::connect(
        crate::sftp::SftpConfig {
            host,
            port,
            user: variable("SE_TASK_SFTP_USER")?,
            auth: crate::sftp::SftpAuth::Password(variable("SE_TASK_SFTP_PASSWORD")?),
            root: base.clone(),
        },
    )?);
    let folder = format!(
        "{}/fuse-task-{}",
        base.trim_end_matches('/'),
        MountId::new_random()?
    );
    raw.mkdir_all(&folder)?;
    let drive = rooted(raw.clone(), &folder, mode)?;
    Ok((raw, drive, folder))
}

/// A variable of the stage's fixtures.
pub(super) fn variable(name: &str) -> io::Result<String> {
    std::env::var(name).map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{name} fehlt (Fixtures der Stufe linux-root: drive-fuse)"),
        )
    })
}

/// Smart Explorer never pins an unknown host key silently: the container's
/// key is pinned only when it is one the fixture scanned into
/// `SE_TASK_SFTP_KNOWN_HOSTS` when it started the container.
pub(super) fn trust_scanned_host_key(host: &str, port: u16) -> io::Result<()> {
    let presented = crate::sftp::probe_host_key(host, port)?;
    if presented.trusted {
        return Ok(());
    }
    let scanned = std::fs::read_to_string(variable("SE_TASK_SFTP_KNOWN_HOSTS")?)?;
    if !scanned_fingerprints(&scanned).contains(&presented.key.fingerprint) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "the SFTP container presents {}, a key the fixture did not scan",
                presented.key.fingerprint
            ),
        ));
    }
    crate::sftp::trust_host_key(host, port, &presented.key.fingerprint, true).map(|_| ())
}

/// `SHA256:<base64 without padding>` (as OpenSSH prints it) of every key of
/// a known_hosts text.
fn scanned_fingerprints(text: &str) -> Vec<String> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
    use base64::Engine;
    use sha2::{Digest, Sha256};

    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| line.split_whitespace().nth(2))
        .filter_map(|key| STANDARD.decode(key).ok())
        .map(|blob| format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(&blob))))
        .collect()
}

/// A read-only folder with names only POSIX can hold, a link and a FIFO.
pub(super) struct NamesBackend;

pub(super) const LISTED_NAMES: [&str; 4] = ["a:b", "CON", "trail.", "plain.txt"];
pub(super) const HIDDEN_NAMES: [&str; 2] = ["link", "fifo"];

impl NamesBackend {
    pub(super) fn drive() -> io::Result<BackendHandle> {
        rooted(Arc::new(Self), "/", MountMode::ReadOnly)
    }

    fn entry(name: &str) -> Option<VfsMeta> {
        let known = LISTED_NAMES.contains(&name) || HIDDEN_NAMES.contains(&name);
        known.then(|| VfsMeta {
            name: name.to_string(),
            is_symlink: name == "link",
            special: name == "fifo",
            size: name.len() as u64,
            mtime_ms: 1_700_000_000_000,
            ..VfsMeta::default()
        })
    }
}

impl Backend for NamesBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn case_sensitive_paths(&self, _root: &str) -> bool {
        true
    }
    fn root_confinement(&self, _root: &str) -> RootConfinement {
        RootConfinement::Enforced
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        if path == "/" {
            return Ok(VfsMeta {
                name: "/".into(),
                is_dir: true,
                ..VfsMeta::default()
            });
        }
        path.strip_prefix('/')
            .and_then(Self::entry)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such fixture name"))
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        if path != "/" {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such folder"));
        }
        Ok(LISTED_NAMES
            .iter()
            .chain(HIDDEN_NAMES.iter())
            .filter_map(|name| Self::entry(name))
            .collect())
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let meta = self.stat(path)?;
        if meta.is_symlink || meta.special {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
        }
        Ok(Box::new(io::Cursor::new(meta.name.into_bytes())))
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(read_only())
    }
    fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> {
        Err(read_only())
    }
    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Err(read_only())
    }
    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(read_only())
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(read_only())
    }
}

fn read_only() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "read-only fixture")
}

/// Which backend call a gate holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Gated {
    /// The content download of this backend path.
    Read(String),
    /// The next upload stage of a save.
    Upload,
}

type HeldGate = (Gated, mpsc::Sender<()>, mpsc::Receiver<bool>);

/// The in-memory fixture with one gate.
pub(super) struct GatedBackend {
    pub(super) inner: Arc<OptimizationBackend>,
    gate: Mutex<Option<HeldGate>>,
}

/// The test side of a gate: the call arrived; release it with success or
/// with a connection failure.
pub(super) struct GateHandle {
    entered: mpsc::Receiver<()>,
    release: mpsc::Sender<bool>,
}

impl GateHandle {
    pub(super) fn entered_within(&self, limit: Duration) -> bool {
        self.entered.recv_timeout(limit).is_ok()
    }

    pub(super) fn release(self, succeed: bool) {
        let _ = self.release.send(succeed);
    }
}

impl GatedBackend {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: OptimizationBackend::new(),
            gate: Mutex::new(None),
        })
    }

    /// The fixture as a drive; every drive of one fixture sees the same
    /// files (a second host after a failed one).
    pub(super) fn drive(self: &Arc<Self>, mode: MountMode) -> io::Result<BackendHandle> {
        rooted(self.clone(), "/", mode)
    }

    pub(super) fn gate(&self, on: Gated) -> GateHandle {
        let (entered_send, entered) = mpsc::channel();
        let (release, release_receive) = mpsc::channel();
        *self.gate.lock().unwrap() = Some((on, entered_send, release_receive));
        GateHandle { entered, release }
    }

    fn pass(&self, call: &Gated) -> io::Result<()> {
        let held = {
            let mut gate = self.gate.lock().unwrap();
            match gate.as_ref() {
                Some((on, _, _)) if on == call => gate.take(),
                _ => None,
            }
        };
        let Some((_, entered, release)) = held else {
            return Ok(());
        };
        let _ = entered.send(());
        match release.recv_timeout(Duration::from_secs(180)) {
            Ok(true) => Ok(()),
            _ => Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "the gated backend call was released with a failure",
            )),
        }
    }
}

impl Backend for GatedBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn parallelism(&self) -> usize {
        self.inner.parallelism()
    }
    fn case_sensitive_paths(&self, root: &str) -> bool {
        self.inner.case_sensitive_paths(root)
    }
    fn root_confinement(&self, root: &str) -> RootConfinement {
        self.inner.root_confinement(root)
    }
    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }
    fn staged_write_capabilities(&self, root: &str) -> StagedWriteCapabilities {
        self.inner.staged_write_capabilities(root)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.open_read_id(path, None)
    }
    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.pass(&Gated::Read(path.to_string()))?;
        self.inner.open_read_id(path, id)
    }
    fn open_read_at(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        self.pass(&Gated::Read(path.to_string()))?;
        self.inner.open_read_at(path, id, offset)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.pass(&Gated::Upload)?;
        self.inner.open_write_new(path)
    }
    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename(source, destination)
    }
    fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename_no_replace(source, destination)
    }
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged(staged, destination)
    }
    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged_no_replace(staged, destination)
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
}
