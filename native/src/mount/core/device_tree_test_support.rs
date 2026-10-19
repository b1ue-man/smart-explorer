//! Fixture of the signals of the device tree: devices that are local
//! folders, a link that counts every connection, and a host-like `Env` that
//! can end and start again over the same cache.
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::super::cache_policy::MountCachePolicy;
use super::super::cache_space::CacheSpaceProbe;
use super::super::device_tree_names::{device_key_of, DEVICES_MOUNT_ID};
use super::super::device_tree_types::{
    ChildConnection, ChildLink, ChildRoot, DeviceKind, DeviceRow, RoomRef, TreeConfig,
};
use super::super::frontend::MountFilesystem;
use super::super::metadata_policy::MountMetadataPolicy;
use super::super::mount_source::PeerMountTarget;
use super::super::path_dialect::PathDialect;
use super::super::spool::prepare_spool_root;
use super::super::types::{BackendRoot, FlushOutcome, MountId, OpenDisposition, OpenFileOptions};
use super::DeviceTree;
use crate::vfs::{
    Backend, DeleteDisposition, LocalBackend, Scheme, StagedWriteCapabilities, VfsMeta, VfsResult,
};

struct Unlimited;

impl CacheSpaceProbe for Unlimited {
    fn available_bytes(&self) -> io::Result<u64> {
        Ok(u64::MAX / 4)
    }
}

/// A shared folder of a device on the local disk; `down` cuts every call.
struct Share {
    base: String,
    inner: LocalBackend,
    down: Arc<AtomicBool>,
}

impl Share {
    fn at(&self, path: &str) -> String {
        if path == "/" {
            self.base.clone()
        } else {
            format!("{}{path}", self.base)
        }
    }

    fn gate(&self) -> io::Result<()> {
        if self.down.load(Ordering::SeqCst) {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        } else {
            Ok(())
        }
    }
}

impl Backend for Share {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.gate()?;
        self.inner.list_dir(&self.at(path))
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.gate()?;
        self.inner.stat(&self.at(path))
    }

    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        self.gate()?;
        self.inner.try_exists(&self.at(path))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.gate()?;
        self.inner.open_read(&self.at(path))
    }

    fn open_read_at(
        &self,
        path: &str,
        _id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        self.gate()?;
        self.inner.open_read_at(&self.at(path), None, offset)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.gate()?;
        self.inner.open_write(&self.at(path))
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.gate()?;
        self.inner.open_write_new(&self.at(path))
    }

    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.gate()?;
        self.inner.rename(&self.at(source), &self.at(destination))
    }

    fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.gate()?;
        self.inner
            .rename_no_replace(&self.at(source), &self.at(destination))
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.gate()?;
        self.inner
            .promote_staged(&self.at(staged), &self.at(destination))
    }

    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.gate()?;
        self.inner
            .promote_staged_no_replace(&self.at(staged), &self.at(destination))
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.gate()?;
        self.inner.remove_file(&self.at(path))
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.gate()?;
        self.inner.remove_dir(&self.at(path))
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.gate()?;
        self.inner.mkdir_all(&self.at(path))
    }

    fn delete_disposition(&self) -> DeleteDisposition {
        self.inner.delete_disposition()
    }

    fn parallelism(&self) -> usize {
        2
    }

    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }

    fn staged_write_capabilities(&self, _root: &str) -> StagedWriteCapabilities {
        self.inner.staged_write_capabilities(&self.base)
    }

    fn case_sensitive_paths(&self, _root: &str) -> bool {
        self.inner.case_sensitive_paths(&self.base)
    }
}

pub(in crate::mount) struct Device {
    dir: PathBuf,
    online: AtomicBool,
    writable: bool,
    shares: Vec<String>,
    pub(in crate::mount) down: Arc<AtomicBool>,
}

struct Lease {
    held: Arc<Mutex<HashSet<String>>>,
    id: String,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut held) = self.held.lock() {
            held.remove(&self.id);
        }
    }
}

#[derive(Default)]
pub(in crate::mount) struct TestLink {
    devices: Mutex<HashMap<String, Arc<Device>>>,
    pub(in crate::mount) connects: AtomicUsize,
    pub(in crate::mount) roots: AtomicUsize,
    held: Arc<Mutex<HashSet<String>>>,
}

impl TestLink {
    fn device(&self, target: &PeerMountTarget) -> io::Result<Arc<Device>> {
        self.devices
            .lock()
            .unwrap()
            .get(&device_key_of(target))
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    pub(in crate::mount) fn leases(&self) -> usize {
        self.held.lock().unwrap().len()
    }
}

impl ChildLink for TestLink {
    fn connect(&self, target: &PeerMountTarget, root: &BackendRoot) -> io::Result<ChildConnection> {
        self.connects.fetch_add(1, Ordering::SeqCst);
        let device = self.device(target)?;
        if !device.online.load(Ordering::SeqCst) {
            return Err(io::Error::from(io::ErrorKind::HostUnreachable));
        }
        let base = device.dir.to_string_lossy().replace('\\', "/");
        let base = if root.as_str() == "/" {
            base
        } else {
            format!("{base}{}", root.as_str())
        };
        Ok(ChildConnection {
            backend: Arc::new(Share {
                base,
                inner: LocalBackend::new("/"),
                down: Arc::clone(&device.down),
            }),
            writable: device.writable,
        })
    }

    fn roots(&self, target: &PeerMountTarget) -> io::Result<Vec<ChildRoot>> {
        self.roots.fetch_add(1, Ordering::SeqCst);
        let device = self.device(target)?;
        if !device.online.load(Ordering::SeqCst) {
            return Err(io::Error::from(io::ErrorKind::HostUnreachable));
        }
        device
            .shares
            .iter()
            .map(|name| {
                Ok(ChildRoot {
                    root: BackendRoot::parse(&format!("/{name}"))?,
                    name: name.clone(),
                })
            })
            .collect()
    }

    fn acquire_lease(&self, _spool_root: &Path, id: &MountId) -> io::Result<Box<dyn Any + Send>> {
        let mut held = self.held.lock().unwrap();
        if !held.insert(id.as_str().to_string()) {
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        }
        Ok(Box::new(Lease {
            held: Arc::clone(&self.held),
            id: id.as_str().to_string(),
        }))
    }
}

/// A device of the fixture.
pub(in crate::mount) struct Spec {
    pub(in crate::mount) id: &'static str,
    pub(in crate::mount) name: &'static str,
    /// `Some((room id, room name))` makes a room device.
    pub(in crate::mount) room: Option<(&'static str, &'static str)>,
    pub(in crate::mount) writable: bool,
    pub(in crate::mount) shares: &'static [&'static str],
}

pub(in crate::mount) fn direct(
    id: &'static str,
    name: &'static str,
    shares: &'static [&'static str],
) -> Spec {
    Spec {
        id,
        name,
        room: None,
        writable: true,
        shares,
    }
}

pub(in crate::mount) struct Env {
    _tmp: tempfile::TempDir,
    pub(in crate::mount) spool: PathBuf,
    pub(in crate::mount) link: Arc<TestLink>,
    tree: Option<Arc<DeviceTree>>,
    pub(in crate::mount) rows: Vec<DeviceRow>,
    dialect: PathDialect,
}

fn new_tree(link: &Arc<TestLink>, spool: &Path, dialect: PathDialect) -> Arc<DeviceTree> {
    let link: Arc<dyn ChildLink> = link.clone();
    DeviceTree::new(
        TreeConfig {
            id: MountId::parse(DEVICES_MOUNT_ID).unwrap(),
            metadata: MountMetadataPolicy::default(),
            cache: MountCachePolicy::default(),
            space: Arc::new(Unlimited),
        },
        link,
        spool,
        dialect,
    )
    .expect("device tree")
}

impl Env {
    pub(in crate::mount) fn new(specs: &[Spec]) -> Self {
        Self::with_dialect(specs, PathDialect::Posix)
    }

    pub(in crate::mount) fn with_dialect(specs: &[Spec], dialect: PathDialect) -> Self {
        let tmp = tempfile::tempdir().expect("temporary folder");
        let spool = prepare_spool_root(&tmp.path().join("spool")).expect("spool");
        std::fs::create_dir_all(spool.join(DEVICES_MOUNT_ID)).expect("tree cache");
        let link = Arc::new(TestLink::default());
        let mut rows = Vec::new();
        for spec in specs {
            let target = match spec.room {
                None => PeerMountTarget::Direct {
                    contact_id: spec.id.into(),
                },
                Some((room, _)) => PeerMountTarget::RoomDevice {
                    room_id: room.into(),
                    device_id: spec.id.into(),
                },
            };
            let key = device_key_of(&target);
            let dir = tmp.path().join("devices").join(spec.id);
            for share in spec.shares {
                std::fs::create_dir_all(dir.join(share)).expect("share");
            }
            link.devices.lock().unwrap().insert(
                key.clone(),
                Arc::new(Device {
                    dir,
                    online: AtomicBool::new(true),
                    writable: spec.writable,
                    shares: spec.shares.iter().map(|share| share.to_string()).collect(),
                    down: Arc::new(AtomicBool::new(false)),
                }),
            );
            rows.push(DeviceRow {
                key,
                name: spec.name.into(),
                kind: if spec.room.is_some() {
                    DeviceKind::Room
                } else {
                    DeviceKind::Direct
                },
                room: spec.room.map(|(id, name)| RoomRef {
                    id: id.into(),
                    name: name.into(),
                }),
                online: true,
                target,
            });
        }
        let tree = new_tree(&link, &spool, dialect);
        Self {
            _tmp: tmp,
            spool,
            link,
            tree: Some(tree),
            rows,
            dialect,
        }
    }

    pub(in crate::mount) fn tree(&self) -> &DeviceTree {
        self.tree.as_ref().expect("tree")
    }

    pub(in crate::mount) fn device(&self, key: &str) -> Arc<Device> {
        Arc::clone(self.link.devices.lock().unwrap().get(key).expect("device"))
    }

    pub(in crate::mount) fn share_file(&self, key: &str, relative: &str) -> PathBuf {
        self.device(key).dir.join(relative)
    }

    pub(in crate::mount) fn publish(&self) {
        self.tree().set_devices(self.rows.clone());
    }

    pub(in crate::mount) fn set_online(&mut self, key: &str, online: bool) {
        self.device(key).online.store(online, Ordering::SeqCst);
        for row in self.rows.iter_mut().filter(|row| row.key == key) {
            row.online = online;
        }
        self.publish();
    }

    /// Ends the host and starts it again over the same cache.
    pub(in crate::mount) fn restart(&mut self) {
        self.tree = None;
        self.tree = Some(new_tree(&self.link, &self.spool, self.dialect));
    }
}

pub(in crate::mount) fn names(list: Vec<VfsMeta>) -> Vec<String> {
    list.into_iter().map(|meta| meta.name).collect()
}

pub(in crate::mount) fn create_options() -> OpenFileOptions {
    OpenFileOptions {
        writable: true,
        disposition: OpenDisposition::CreateNew,
    }
}

pub(in crate::mount) fn open_existing() -> OpenFileOptions {
    OpenFileOptions {
        writable: false,
        disposition: OpenDisposition::OpenExisting,
    }
}

pub(in crate::mount) fn create_file(tree: &DeviceTree, path: &str, bytes: &[u8]) -> io::Result<()> {
    let handle = tree.open_file(path, create_options())?;
    tree.write(handle, 0, bytes)?;
    let flushed = tree.flush(handle);
    tree.close(handle)?;
    match flushed? {
        FlushOutcome::Committed | FlushOutcome::NoChanges => Ok(()),
        other => Err(io::Error::other(format!("{other:?}"))),
    }
}

pub(in crate::mount) fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(Instant::now() < deadline, "timeout waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub(in crate::mount) const LAPTOP: &str = "direct~a";
