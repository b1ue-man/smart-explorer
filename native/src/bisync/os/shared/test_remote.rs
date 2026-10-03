//! Test backend: a remote-like view of a local folder below `/data` with its
//! own namespace and flow (like a second server), call counters, optional
//! delays that make concurrency visible, a per-operation hook, injectable
//! write failures, overload replies and stale listings, and a naive, racy
//! folder creation (like a plain SFTP client). It reports
//! `parallelism() == 1`, as SFTP and FTP did, so tests show where the flows
//! now run work concurrently.
use crate::vfs::{Backend, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const REMOTE_ROOT: &str = "/data";
/// The delay an injected overload reply asks for (short: tests stay fast).
pub(crate) const OVERLOAD_RETRY_AFTER: Duration = Duration::from_millis(5);

static NEXT_REMOTE: AtomicUsize = AtomicUsize::new(0);

/// Counted calls of one fake remote.
#[derive(Default)]
pub(crate) struct Calls {
    pub(crate) stat: AtomicUsize,
    pub(crate) list: AtomicUsize,
    pub(crate) mkdir_all: AtomicUsize,
    pub(crate) create_dir: AtomicUsize,
    pub(crate) reads: AtomicUsize,
    pub(crate) peak_reads: AtomicUsize,
    pub(crate) peak_lists: AtomicUsize,
    /// Injected overload replies given.
    pub(crate) congested: AtomicUsize,
    /// Every `stat` path, in call order.
    pub(crate) stat_paths: Mutex<Vec<String>>,
    open_reads: AtomicUsize,
    open_lists: AtomicUsize,
}

impl Calls {
    pub(crate) fn stats_of(&self, path: &str) -> usize {
        self.stat_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|seen| *seen == path)
            .count()
    }
}

pub(crate) type Hook = Arc<dyn Fn(&str, &str) + Send + Sync>;

pub(crate) struct FakeRemote {
    inner: LocalBackend,
    base: String,
    identity: String,
    ceiling: Option<usize>,
    delay: Duration,
    fail_writes_to: Option<String>,
    racy_folders: bool,
    hook: Option<Hook>,
    scheme: Option<Scheme>,
    /// Operation → overload replies still to give before it succeeds.
    overload: Mutex<HashMap<&'static str, usize>>,
    /// A listed file whose directory entry shows a stale size.
    stale: Option<(String, u64)>,
    pub(crate) calls: Arc<Calls>,
}

impl FakeRemote {
    /// `/data` of a server called `host`, stored in the local folder `base`.
    pub(crate) fn new(base: &Path, host: &str) -> Self {
        let base = base.to_string_lossy().replace('\\', "/");
        let unique = NEXT_REMOTE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        Self {
            inner: LocalBackend::new(&base),
            base: base.trim_end_matches('/').to_string(),
            identity: format!("fake-remote:{host}:{}:{unique}:{nanos}", std::process::id()),
            ceiling: None,
            delay: Duration::ZERO,
            fail_writes_to: None,
            racy_folders: false,
            hook: None,
            scheme: None,
            overload: Mutex::new(HashMap::new()),
            stale: None,
            calls: Arc::new(Calls::default()),
        }
    }

    /// At most `ceiling` concurrent transfers on this connection.
    pub(crate) fn with_ceiling(mut self, ceiling: usize) -> Self {
        self.ceiling = Some(ceiling);
        self
    }

    /// Every listing and every opened reader takes this long first.
    pub(crate) fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Writing a stage of a file whose name starts with `name` fails.
    pub(crate) fn failing_writes_to(mut self, name: &str) -> Self {
        self.fail_writes_to = Some(name.to_string());
        self
    }

    /// Folder creation probes, pauses and creates; a folder that appeared
    /// meanwhile is an error, like a naive protocol client.
    pub(crate) fn with_racy_folders(mut self) -> Self {
        self.racy_folders = true;
        self
    }

    /// Called with (operation, path) before every listing and read.
    pub(crate) fn with_hook(mut self, hook: Hook) -> Self {
        self.hook = Some(hook);
        self
    }

    /// Reports `scheme` (default: that of the local folder).
    pub(crate) fn with_scheme(mut self, scheme: Scheme) -> Self {
        self.scheme = Some(scheme);
        self
    }

    /// The first `times` calls of `operation` ("list", "stat", "read",
    /// "write") are refused as overload (`vfs::congestion_error`).
    pub(crate) fn with_overload(self, operation: &'static str, times: usize) -> Self {
        self.overload
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(operation, times);
        self
    }

    /// Listings show `size` for the file `name` (`stat` shows the truth), like
    /// the stale directory entry of a hard-linked file on NTFS or SMB.
    pub(crate) fn with_stale_listing(mut self, name: &str, size: u64) -> Self {
        self.stale = Some((name.to_string(), size));
        self
    }

    fn overloaded(&self, operation: &str) -> VfsResult<()> {
        let mut overload = self
            .overload
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match overload.get_mut(operation) {
            Some(left) if *left > 0 => {
                *left -= 1;
                self.calls.congested.fetch_add(1, Ordering::SeqCst);
                Err(crate::vfs::congestion_error(
                    "injected overload",
                    Some(OVERLOAD_RETRY_AFTER),
                ))
            }
            _ => Ok(()),
        }
    }

    fn real(&self, path: &str) -> io::Result<String> {
        let rest = path
            .strip_prefix(REMOTE_ROOT)
            .filter(|rest| rest.is_empty() || rest.starts_with('/'))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("outside the fake remote: {path}"),
                )
            })?;
        Ok(format!("{}{rest}", self.base))
    }

    fn call_hook(&self, operation: &str, path: &str) {
        if let Some(hook) = &self.hook {
            hook(operation, path);
        }
    }

    /// Probes, pauses, then creates one level (with `parents` the missing
    /// parents first): two concurrent creators of one folder collide.
    fn racy_create(&self, path: &str, parents: bool) -> VfsResult<()> {
        let real = self.real(path)?;
        if Path::new(&real).is_dir() {
            return Ok(());
        }
        if parents {
            if let Some((parent, _)) = path.rsplit_once('/') {
                if parent.len() > REMOTE_ROOT.len() {
                    self.racy_create(parent, true)?;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(30));
        std::fs::create_dir(real)
    }
}

struct Tracked {
    reader: Box<dyn Read + Send>,
    calls: Arc<Calls>,
}

impl Read for Tracked {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buffer)
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.calls.open_reads.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Backend for FakeRemote {
    fn scheme(&self) -> Scheme {
        self.scheme.unwrap_or_else(|| self.inner.scheme())
    }

    fn root_display(&self) -> String {
        REMOTE_ROOT.to_string()
    }

    fn state_identity(&self) -> String {
        self.identity.clone()
    }

    fn namespace_identity(&self) -> String {
        self.identity.clone()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.calls.list.fetch_add(1, Ordering::SeqCst);
        self.call_hook("list", path);
        self.overloaded("list")?;
        let open = self.calls.open_lists.fetch_add(1, Ordering::SeqCst) + 1;
        self.calls.peak_lists.fetch_max(open, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        let result = self.real(path).and_then(|real| self.inner.list_dir(&real));
        self.calls.open_lists.fetch_sub(1, Ordering::SeqCst);
        let mut entries = result?;
        if let Some((name, size)) = &self.stale {
            for entry in entries.iter_mut().filter(|entry| &entry.name == name) {
                entry.size = *size;
            }
        }
        Ok(entries)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.calls.stat.fetch_add(1, Ordering::SeqCst);
        self.calls
            .stat_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(path.to_string());
        self.overloaded("stat")?;
        self.inner.stat(&self.real(path)?)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.calls.reads.fetch_add(1, Ordering::SeqCst);
        self.call_hook("read", path);
        self.overloaded("read")?;
        let open = self.calls.open_reads.fetch_add(1, Ordering::SeqCst) + 1;
        self.calls.peak_reads.fetch_max(open, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        match self.real(path).and_then(|real| self.inner.open_read(&real)) {
            Ok(reader) => Ok(Box::new(Tracked {
                reader,
                calls: self.calls.clone(),
            })),
            Err(error) => {
                self.calls.open_reads.fetch_sub(1, Ordering::SeqCst);
                Err(error)
            }
        }
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.overloaded("write")?;
        let name = path.rsplit('/').next().unwrap_or(path);
        if let Some(failing) = &self.fail_writes_to {
            if name.starts_with(failing.as_str()) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected write failure",
                ));
            }
        }
        self.inner.open_write(&self.real(path)?)
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.overloaded("write")?;
        self.call_hook("write", path);
        if self.fail_writes_to.as_ref().is_some_and(|name| {
            path.rsplit('/')
                .next()
                .unwrap_or(path)
                .contains(name.as_str())
        }) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected write failure",
            ));
        }
        self.inner.open_write_new(&self.real(path)?)
    }

    fn open_write_copy_stage(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.open_write_new(path)
    }

    fn discard_copy_stage(&self, path: &str) -> VfsResult<()> {
        self.inner.discard_copy_stage(&self.real(path)?)
    }

    fn extensions(&self) -> Option<&dyn crate::vfs::BackendExtensions> {
        Some(self)
    }

    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner
            .rename(&self.real(source)?, &self.real(destination)?)
    }

    fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner
            .rename_no_replace(&self.real(source)?, &self.real(destination)?)
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner
            .promote_staged(&self.real(staged)?, &self.real(destination)?)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(&self.real(path)?)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(&self.real(path)?)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.calls.mkdir_all.fetch_add(1, Ordering::SeqCst);
        if self.racy_folders {
            return self.racy_create(path, true);
        }
        self.inner.mkdir_all(&self.real(path)?)
    }

    fn create_dir(&self, path: &str) -> VfsResult<()> {
        self.calls.create_dir.fetch_add(1, Ordering::SeqCst);
        if self.racy_folders {
            return self.racy_create(path, false);
        }
        self.inner.create_dir(&self.real(path)?)
    }

    fn parallelism(&self) -> usize {
        1
    }

    fn flow_key(&self, _path: &str) -> String {
        self.identity.clone()
    }

    fn transfer_ceiling(&self, _path: &str) -> Option<usize> {
        self.ceiling
    }

    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }
}

impl crate::vfs::BackendExtensions for FakeRemote {
    fn open_read_regular(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.calls.reads.fetch_add(1, Ordering::SeqCst);
        self.call_hook("read", path);
        self.overloaded("read")?;
        let open = self.calls.open_reads.fetch_add(1, Ordering::SeqCst) + 1;
        self.calls.peak_reads.fetch_max(open, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        let result = self
            .real(path)
            .and_then(|real| crate::vfs::open_read_regular(&self.inner, &real, id));
        match result {
            Ok(reader) => Ok(Box::new(Tracked {
                reader,
                calls: self.calls.clone(),
            })),
            Err(error) => {
                self.calls.open_reads.fetch_sub(1, Ordering::SeqCst);
                Err(error)
            }
        }
    }

    fn open_write_copy_stage_timed(
        &self,
        path: &str,
        size: u64,
        _mtime_ms: i64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.open_write_copy_stage_sized(path, size)
    }

    fn finish_stage(
        &self,
        path: &str,
        finish: crate::vfs::StageFinish,
    ) -> VfsResult<crate::vfs::StageFinished> {
        crate::vfs::finish_stage(&self.inner, &self.real(path)?, finish)
    }

    fn sync_filesystem(&self, root: &str) -> VfsResult<bool> {
        crate::vfs::sync_filesystem(&self.inner, &self.real(root)?)
    }

    fn target_limits(&self, root: &str) -> crate::vfs::TargetLimits {
        self.real(root)
            .map(|real| crate::vfs::target_limits(&self.inner, &real))
            .unwrap_or_default()
    }

    fn unix_mode(&self, path: &str) -> VfsResult<Option<u32>> {
        crate::vfs::unix_mode(&self.inner, &self.real(path)?)
    }

    fn volume_identity(&self, root: &str) -> VfsResult<Option<crate::vfs::VolumeIdentity>> {
        crate::vfs::volume_identity(&self.inner, &self.real(root)?)
    }
}
